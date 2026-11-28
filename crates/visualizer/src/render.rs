// SPDX-License-Identifier: GPL-3.0-only
//! The render thread: a wgpu device on the attached window, the Lactea engine, the preset playlist and the audio
//! from the scope.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crossbeam_channel::Receiver;
use infrasonic_engine::output::OutputSpec;
use infrasonic_engine::scope::{Scope, ScopeReader};
use lactea::{Engine, EngineConfig, FrameClock, Playlist, Transition};
use parking_lot::Mutex;
use wgpu::rwh::{DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle};

use crate::playlist::{MAX_PRESET_BYTES, preset_name};
use crate::{Collections, Limits, Request, Settings, Shared};

/// Audio kept for the next frame at most; anything older is dropped (the reader fell behind, e.g. after a stall).
const MAX_PCM_SECONDS: f64 = 0.25;
/// Ring between the output callback and this thread.
const SCOPE_SECONDS: f64 = 1.0;
/// Sample rate assumed until the first output stream is opened.
const DEFAULT_RATE: u32 = 48_000;
/// Presets tried in a row before the playlist gives up on a switch (projectM's default is 3; broken presets are
/// rare in the curated collections).
const LOAD_RETRIES: u32 = 8;

/// The display handle of the window, for the instance (GLES needs it to present).
#[derive(Debug)]
struct DisplayOf<W>(Arc<W>);

impl<W: HasDisplayHandle> HasDisplayHandle for DisplayOf<W> {
    fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
        self.0.display_handle()
    }
}

pub(crate) fn run<W>(
    window: W,
    width: u32,
    height: u32,
    scope: Arc<Scope>,
    shared: Arc<Mutex<Shared>>,
    session: u64,
    requests: Receiver<Request>,
) where
    W: HasWindowHandle + HasDisplayHandle + std::fmt::Debug + Send + Sync + 'static,
{
    if let Some(mut s) = current(&shared, session) {
        s.error = None;
    }
    let result = Gpu::new(Arc::new(window), width, height)
        .and_then(|gpu| Session::new(gpu, &scope, &shared, session));
    let outcome = match result {
        Ok(mut s) => s.run(&shared, &requests),
        Err(e) => Err(e),
    };
    if let Err(e) = &outcome {
        tracing::warn!("visualizer stopped: {e}");
    }
    let Some(mut s) = current(&shared, session) else {
        return;
    };
    s.current = None;
    if let Err(e) = outcome {
        s.error = Some(e);
    }
}

/// The shared state, if `session` is still the latest attach (an older render thread no longer reports).
fn current(shared: &Mutex<Shared>, session: u64) -> Option<parking_lot::MutexGuard<'_, Shared>> {
    let s = shared.lock();
    (s.session == session).then_some(s)
}

struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    lost: Arc<AtomicBool>,
}

impl Gpu {
    fn new<W>(window: Arc<W>, width: u32, height: u32) -> Result<Self, String>
    where
        W: HasWindowHandle + HasDisplayHandle + std::fmt::Debug + Send + Sync + 'static,
    {
        let mut desc = wgpu::InstanceDescriptor::new_with_display_handle(Box::new(DisplayOf(
            Arc::clone(&window),
        )));
        desc.backends = wgpu::Backends::VULKAN | wgpu::Backends::GL;
        // No shader debug info in any build, as in release builds: with it, naga puts the WGSL source into every
        // SPIR-V module (OpSource language 10), which the Android emulator's SwiftShader crashed on.
        let mut flags = wgpu::InstanceFlags::from_build_config().with_env();
        flags.remove(wgpu::InstanceFlags::DEBUG);
        desc.flags = flags;
        let instance = wgpu::Instance::new(desc);
        let surface = instance
            .create_surface(window)
            .map_err(|e| format!("cannot create the surface: {e}"))?;
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            compatible_surface: Some(&surface),
            ..Default::default()
        }))
        .map_err(|e| format!("no graphics adapter: {e}"))?;
        let info = adapter.get_info();
        tracing::info!("visualizer: {} ({:?})", info.name, info.backend);
        // What the adapter has: GLES devices do not reach WebGPU's default limits, and the engine checks sizes
        // against these.
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("infrasonic visualizer"),
            required_limits: adapter.limits(),
            ..Default::default()
        }))
        .map_err(|e| format!("cannot open the graphics device: {e}"))?;
        // A validation error must not abort the app; the engine checks presets with error scopes of its own.
        device.on_uncaptured_error(Arc::new(|e| tracing::error!("wgpu: {e}")));
        let lost = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&lost);
        device.set_device_lost_callback(move |reason, msg| {
            tracing::warn!("graphics device lost ({reason:?}): {msg}");
            flag.store(true, Ordering::Release);
        });
        let caps = surface.get_capabilities(&adapter);
        let mut config = surface
            .get_default_config(&adapter, width.max(1), height.max(1))
            .ok_or("the surface does not work with this adapter")?;
        // The engine writes its gamma-free image as is: an sRGB surface would brighten it.
        use wgpu::TextureFormat as F;
        if let Some(f) = [F::Rgba8Unorm, F::Bgra8Unorm]
            .into_iter()
            .find(|f| caps.formats.contains(f))
        {
            config.format = f;
        }
        config.present_mode = wgpu::PresentMode::Fifo;
        config.desired_maximum_frame_latency = 2;
        if caps.alpha_modes.contains(&wgpu::CompositeAlphaMode::Opaque) {
            config.alpha_mode = wgpu::CompositeAlphaMode::Opaque;
        }
        surface.configure(&device, &config);
        Ok(Self {
            device,
            queue,
            surface,
            config,
            lost,
        })
    }

    fn resize(&mut self, width: u32, height: u32) -> bool {
        if width == 0 || height == 0 || (width, height) == (self.config.width, self.config.height) {
            return false;
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.device, &self.config);
        true
    }

    fn acquire(&mut self) -> Option<wgpu::SurfaceTexture> {
        use wgpu::CurrentSurfaceTexture as C;
        match self.surface.get_current_texture() {
            C::Success(t) => Some(t),
            C::Suboptimal(t) => {
                self.surface.configure(&self.device, &self.config);
                Some(t)
            }
            C::Timeout | C::Occluded => None,
            C::Outdated | C::Lost | C::Validation => {
                self.surface.configure(&self.device, &self.config);
                None
            }
        }
    }
}

struct Session {
    gpu: Gpu,
    engine: Engine,
    spec: OutputSpec,
    reader: ScopeReader,
    pcm: Vec<f32>,
    playlist: Playlist,
    generation: u64,
    settings: Settings,
    limits: Limits,
    collections: Collections,
    start: Instant,
    frame: u32,
    shown_since: Instant,
    /// The attach this session renders for ([`Shared::session`]).
    session: u64,
}

impl Session {
    fn new(
        gpu: Gpu,
        scope: &Arc<Scope>,
        shared: &Mutex<Shared>,
        session: u64,
    ) -> Result<Self, String> {
        let (collections, settings, limits, generation) = {
            let s = shared.lock();
            (
                s.collections.clone(),
                s.settings.clone(),
                s.limits,
                s.generation,
            )
        };
        let spec = scope.format().unwrap_or(OutputSpec {
            sample_rate: DEFAULT_RATE,
            channels: 2,
        });
        let capacity =
            (f64::from(spec.sample_rate) * SCOPE_SECONDS) as usize * spec.channels.max(1);
        let reader = scope.open(capacity);
        let engine = new_engine(gpu.target(), spec, &settings, &limits, &collections)?;
        let now = Instant::now();
        let mut session = Self {
            gpu,
            engine,
            spec,
            reader,
            pcm: Vec::new(),
            playlist: new_playlist(&collections),
            generation,
            settings,
            limits,
            collections,
            start: now,
            frame: 0,
            shown_since: now,
            session,
        };
        session.switch(Switch::Next, true, shared);
        Ok(session)
    }

    fn run(&mut self, shared: &Mutex<Shared>, requests: &Receiver<Request>) -> Result<(), String> {
        let mut next_frame = Instant::now();
        loop {
            while let Ok(r) = requests.try_recv() {
                match r {
                    Request::Stop => return Ok(()),
                    Request::Resize(w, h) => {
                        if self.gpu.resize(w, h) {
                            self.engine
                                .resize(w, h)
                                .map_err(|e| format!("cannot resize: {e}"))?;
                        }
                    }
                    Request::Next => self.switch(Switch::Next, false, shared),
                    Request::Previous => self.switch(Switch::Previous, false, shared),
                }
            }
            if self.gpu.lost.load(Ordering::Acquire) {
                return Err("the graphics device was lost".into());
            }
            self.follow_shared(shared)?;
            let preset_seconds = f64::from(self.settings.preset_seconds);
            if preset_seconds > 0.0 && self.shown_since.elapsed().as_secs_f64() >= preset_seconds {
                self.switch(Switch::Next, false, shared);
            }
            self.feed_audio()?;
            if let Some(frame) = self.gpu.acquire() {
                let view = frame.texture.create_view(&Default::default());
                let mut enc = self.gpu.device.create_command_encoder(&Default::default());
                let clock = FrameClock {
                    time: self.start.elapsed().as_secs_f64(),
                    frame: self.frame,
                };
                self.engine.render(&mut enc, &view, clock);
                self.gpu.queue.submit(Some(enc.finish()));
                self.gpu.queue.present(frame);
                self.frame = self.frame.wrapping_add(1);
            }
            // Fifo presents at the display's rate; a lower `fps` waits for its own frame time.
            let period = Duration::from_secs_f64(1.0 / f64::from(self.settings.fps.clamp(1, 120)));
            next_frame += period;
            let now = Instant::now();
            if next_frame > now {
                std::thread::sleep(next_frame - now);
            } else {
                next_frame = now;
            }
        }
    }

    /// Take up changes the host made: collections and limits rebuild the engine, settings apply at once.
    fn follow_shared(&mut self, shared: &Mutex<Shared>) -> Result<(), String> {
        let (collections, settings, limits, generation) = {
            let s = shared.lock();
            if s.generation == self.generation {
                return Ok(());
            }
            (
                s.collections.clone(),
                s.settings.clone(),
                s.limits,
                s.generation,
            )
        };
        self.generation = generation;
        let rebuild = collections != self.collections || limits != self.limits;
        self.settings = settings;
        if rebuild {
            self.collections = collections;
            self.limits = limits;
            self.engine = new_engine(
                self.gpu.target(),
                self.spec,
                &self.settings,
                &self.limits,
                &self.collections,
            )?;
            self.playlist = new_playlist(&self.collections);
            self.switch(Switch::Next, true, shared);
        }
        Ok(())
    }

    /// Hand the audio that was played since the last frame to the engine; a new stream format rebuilds it (the
    /// analysis depends on the sample rate).
    fn feed_audio(&mut self) -> Result<(), String> {
        self.pcm.clear();
        self.reader.read(&mut self.pcm);
        if let Some(spec) = self.reader.format()
            && spec != self.spec
            && spec.channels > 0
        {
            self.spec = spec;
            self.pcm.clear();
            self.engine = new_engine(
                self.gpu.target(),
                spec,
                &self.settings,
                &self.limits,
                &self.collections,
            )?;
            self.reload_current();
            return Ok(());
        }
        let channels = self.spec.channels.max(1);
        let keep = (f64::from(self.spec.sample_rate) * MAX_PCM_SECONDS) as usize * channels;
        let frames_start = self.pcm.len().saturating_sub(keep) / channels * channels;
        if self.pcm.len() > frames_start {
            self.engine
                .push_pcm_f32(&self.pcm[frames_start..], channels);
        }
        Ok(())
    }

    fn reload_current(&mut self) {
        let position = self.playlist.position();
        let engine = &mut self.engine;
        self.playlist.set_position(position, true, |req| {
            load(engine, req.filename, Transition::Cut)
        });
    }

    fn switch(&mut self, to: Switch, cut: bool, shared: &Mutex<Shared>) {
        if self.playlist.is_empty() {
            return;
        }
        let transition = if cut || self.settings.transition_seconds <= 0.0 {
            Transition::Cut
        } else {
            Transition::Soft {
                seconds: f64::from(self.settings.transition_seconds),
            }
        };
        let engine = &mut self.engine;
        let loader = |req: lactea::LoadRequest<'_>| load(engine, req.filename, transition);
        match to {
            Switch::Next => self.playlist.play_next(cut, loader),
            Switch::Previous => self.playlist.play_last(cut, loader),
        };
        self.shown_since = Instant::now();
        let current = self
            .playlist
            .item(self.playlist.position())
            .map(|p| preset_name(std::path::Path::new(p)));
        if let Some(mut s) = self::current(shared, self.session) {
            s.current = current;
            s.preset_count = self.playlist.len() as usize;
        }
    }
}

#[derive(Clone, Copy)]
enum Switch {
    Next,
    Previous,
}

pub(crate) fn load(engine: &mut Engine, path: &str, transition: Transition) -> Result<(), String> {
    let bytes = read_preset(std::path::Path::new(path))?;
    let name = preset_name(std::path::Path::new(path));
    engine
        .load_preset(&bytes, &name, transition)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

fn read_preset(path: &std::path::Path) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    file.take(MAX_PRESET_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_PRESET_BYTES {
        return Err("preset file too large".into());
    }
    Ok(bytes)
}

/// The target an engine renders into: device, queue, size and format.
pub(crate) struct Target<'a> {
    pub(crate) device: &'a wgpu::Device,
    pub(crate) queue: &'a wgpu::Queue,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) format: wgpu::TextureFormat,
}

impl Gpu {
    fn target(&self) -> Target<'_> {
        Target {
            device: &self.device,
            queue: &self.queue,
            width: self.config.width,
            height: self.config.height,
            format: self.config.format,
        }
    }
}

pub(crate) fn new_engine(
    target: Target<'_>,
    spec: OutputSpec,
    settings: &Settings,
    limits: &Limits,
    collections: &Collections,
) -> Result<Engine, String> {
    let cfg = EngineConfig {
        width: target.width,
        height: target.height,
        output_format: target.format,
        sample_rate: spec.sample_rate,
        fps: settings.fps.clamp(1, 120),
        auto_compat: true,
        // the host decides when to change presets
        preset_duration: 1e9,
        loop_budget: Some(limits.loop_budget),
        memory_budget: Some(limits.memory_budget),
        shader_loop_budget: Some(limits.shader_loop_budget),
        ..EngineConfig::default()
    };
    let mut engine = Engine::new(target.device, target.queue, cfg)
        .map_err(|e| format!("cannot start the engine: {e}"))?;
    let textures = lactea_host::DirTextures::new(&collections.texture_dirs())
        .with_decode_limit(limits.texture_bytes);
    engine.set_texture_provider(Box::new(textures));
    Ok(engine)
}

fn new_playlist(collections: &Collections) -> Playlist {
    let seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(1, |d| d.as_nanos() as u64);
    let mut playlist = Playlist::new(seed);
    let names: Vec<String> = collections
        .presets()
        .iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    playlist.add_presets(&names, false);
    playlist.set_shuffle(true);
    playlist.set_retry_count(LOAD_RETRIES);
    playlist
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A headless device, or `None` (no adapter: the test is skipped).
    fn device() -> Option<(wgpu::Device, wgpu::Queue)> {
        let instance = wgpu::Instance::default();
        let adapter =
            pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
                .ok()?;
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).ok()
    }

    const PRESET: &str = "[preset00]\nMILKDROP_PRESET_VERSION=201\nPSVERSION=2\nPSVERSION_WARP=2\nPSVERSION_COMP=2\n\
        zoom=1.01\nwave_r=1\nwave_g=0.5\nwave_b=0.2\nwave_a=1\nper_frame_1=wave_r = 0.5 + 0.5*sin(time);\n\
        comp_1=`shader_body { ret = tex2D(sampler_main, uv).xyz + float3(0.2, 0.1, 0.3); }\n";

    #[test]
    fn renders_a_collection_preset_within_the_phone_limits() {
        let Some((device, queue)) = device() else {
            eprintln!("SKIP: no wgpu adapter");
            return;
        };
        let dir = tempfile::tempdir().expect("tmp");
        std::fs::create_dir_all(dir.path().join("presets/Test")).expect("mkdir");
        let path = dir.path().join("presets/Test/glow.milk");
        std::fs::write(&path, PRESET).expect("write");
        let collections = Collections::scan(&[dir.path().into()]);
        assert_eq!(collections.presets().len(), 1);
        let (w, h) = (64, 36);
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        let target = Target {
            device: &device,
            queue: &queue,
            width: w,
            height: h,
            format: wgpu::TextureFormat::Rgba8Unorm,
        };
        let spec = OutputSpec {
            sample_rate: 48_000,
            channels: 2,
        };
        let mut engine = new_engine(
            target,
            spec,
            &Settings::default(),
            &Limits::phone(),
            &collections,
        )
        .expect("engine");
        load(&mut engine, &path.to_string_lossy(), Transition::Cut).expect("load");
        let pcm: Vec<f32> = (0..1600 * 2)
            .map(|i| ((i as f32) * 0.03).sin() * 0.5)
            .collect();
        for frame in 0..10 {
            engine.push_pcm_f32(&pcm, 2);
            let mut enc = device.create_command_encoder(&Default::default());
            engine.render(
                &mut enc,
                &view,
                FrameClock {
                    time: f64::from(frame) / 30.0,
                    frame,
                },
            );
            queue.submit(Some(enc.finish()));
        }
        // read the last frame back: the composite shader adds a colour, so it is not black
        let row = (w * 4).div_ceil(256) * 256;
        let buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: u64::from(row * h),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut enc = device.create_command_encoder(&Default::default());
        enc.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buf,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: None,
                },
            },
            texture.size(),
        );
        queue.submit(Some(enc.finish()));
        buf.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        let _ = device.poll(wgpu::PollType::wait_indefinitely());
        let data = buf.slice(..).get_mapped_range().expect("mapped");
        assert!(
            data.chunks(4).any(|px| px[0] > 30 && px[2] > 50),
            "the frame is black"
        );
        let usage = engine.expression_usage().expect("usage");
        assert!(!usage.loops_cut && !usage.memory_refused);
    }
}
