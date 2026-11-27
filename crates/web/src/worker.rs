// SPDX-License-Identifier: GPL-3.0-only
//! The audio worker: runs the engine in pump mode, downloads songs and feeds the AudioWorklet
//! through a `MessagePort`. Reports from the worklet (about every 20 ms) drive the pump, so
//! playback keeps going in background tabs where timers are throttled.

use std::cell::RefCell;
use std::sync::Arc;

use infrasonic_engine::output::{OutputSpec, PullHandle, PullOutputFactory};
use infrasonic_engine::{Engine, EngineConfig, Event, PlayState, Pump};
use js_sys::{Array, Float32Array, Object, Reflect, Uint8Array};
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use web_sys::{DedicatedWorkerGlobalScope, MessageEvent, MessagePort, Response};

use crate::protocol::{FromWorker, ToWorker, parse};
use crate::sources::Sources;

/// Audio the worklet should hold, in seconds: enough to bridge a busy main thread.
const AHEAD: f64 = 0.3;
/// Samples per message to the worklet (stereo frames × 2).
const BLOCK: usize = 2048 * 2;

struct State {
    engine: Engine,
    pump: Pump,
    out: PullHandle,
    sources: Arc<Sources>,
    worklet: MessagePort,
    rate: u32,
    /// Frames sent to the worklet and frames it reported as played, since the last flush.
    posted: u64,
    heard: u64,
    /// Flush generation; reports from before a flush are ignored.
    generation: u32,
    paused: bool,
    scheduled: bool,
    last_status: f64,
    buf: Vec<f32>,
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
    static PENDING: RefCell<Vec<ToWorker>> = const { RefCell::new(Vec::new()) };
}

fn scope() -> DedicatedWorkerGlobalScope {
    js_sys::global().unchecked_into()
}

fn post(m: &FromWorker) {
    let _ = scope().post_message(&JsValue::from_str(&m.to_json()));
}

fn now_ms() -> f64 {
    js_sys::Date::now()
}

/// Entry point of the worker program.
pub fn start() {
    console_error_panic_hook::set_once();
    let on_message = Closure::<dyn FnMut(MessageEvent)>::new(|e: MessageEvent| {
        let data = e.data();
        // `{ msg: "<json>", port }` for Init, a JSON string otherwise.
        let (text, port) = if let Some(s) = data.as_string() {
            (s, None)
        } else {
            let msg = Reflect::get(&data, &"msg".into())
                .ok()
                .and_then(|v| v.as_string());
            let port = Reflect::get(&data, &"port".into())
                .ok()
                .and_then(|p| p.dyn_into::<MessagePort>().ok());
            (msg.unwrap_or_default(), port)
        };
        let Some(msg) = parse::<ToWorker>(&text) else {
            return;
        };
        match (msg, port) {
            (
                ToWorker::Init {
                    sample_rate,
                    volume,
                    replay_gain,
                },
                Some(port),
            ) => init(sample_rate, volume, replay_gain, port),
            (m, _) => handle(m),
        }
    });
    scope().set_onmessage(Some(on_message.as_ref().unchecked_ref()));
    on_message.forget();
}

fn init(
    rate: u32,
    volume: u8,
    replay_gain: infrasonic_engine::ReplayGainConfig,
    worklet: MessagePort,
) {
    let (factory, out) = PullOutputFactory::new(OutputSpec {
        sample_rate: rate,
        channels: 2,
    });
    let sources = Arc::new(Sources::default());
    let config = EngineConfig {
        volume,
        replay_gain,
        ..EngineConfig::default()
    };
    let (engine, pump) = Engine::new_pump(config, sources.clone(), Box::new(factory));
    let on_report = Closure::<dyn FnMut(MessageEvent)>::new(|e: MessageEvent| {
        let d = e.data();
        let num = |k: &str| {
            Reflect::get(&d, &k.into())
                .ok()
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0)
        };
        with_state(|s| {
            if num("gen") as u32 == s.generation {
                s.heard = num("heard") as u64;
                if num("starved") > 0.0 {
                    s.out.note_underrun();
                }
            }
        });
        tick();
    });
    worklet.set_onmessage(Some(on_report.as_ref().unchecked_ref()));
    on_report.forget();
    worklet.start();
    STATE.with(|st| {
        *st.borrow_mut() = Some(State {
            engine,
            pump,
            out,
            sources,
            worklet,
            rate,
            posted: 0,
            heard: 0,
            generation: 0,
            paused: false,
            scheduled: false,
            last_status: 0.0,
            buf: vec![0.0; BLOCK],
        })
    });
    for m in PENDING.with(|p| std::mem::take(&mut *p.borrow_mut())) {
        handle(m);
    }
    tick();
}

fn with_state<R>(f: impl FnOnce(&mut State) -> R) -> Option<R> {
    STATE.with(|st| st.borrow_mut().as_mut().map(f))
}

fn handle(m: ToWorker) {
    let handled = with_state(|s| match m.clone() {
        ToWorker::Init { .. } => {}
        ToWorker::Urls { urls } => s.sources.set_urls(urls),
        ToWorker::Command { cmd } => s.engine.send(cmd),
    });
    if handled.is_none() {
        PENDING.with(|p| p.borrow_mut().push(m));
        return;
    }
    tick();
}

/// One pump turn, feeding the worklet and reporting to the page. Schedules itself while
/// there is work.
fn tick() {
    let again = with_state(|s| {
        s.scheduled = false;
        s.pump.turn();
        feed(s);
        start_downloads(s);
        report(s);
        s.pump.busy() || s.pump.waiting() > 0
    });
    if again == Some(true) {
        schedule(1);
    }
}

fn schedule(ms: i32) {
    let already = with_state(|s| std::mem::replace(&mut s.scheduled, true));
    if already != Some(false) {
        return;
    }
    let cb = Closure::once_into_js(tick);
    let _ = scope().set_timeout_with_callback_and_timeout_and_arguments_0(cb.unchecked_ref(), ms);
}

fn send_worklet(s: &State, msg: &JsValue, transfer: Option<&JsValue>) {
    let r = match transfer {
        Some(t) => s
            .worklet
            .post_message_with_transferable(msg, &Array::of1(t)),
        None => s.worklet.post_message(msg),
    };
    if let Err(e) = r {
        web_sys::console::warn_1(&e);
    }
}

fn obj(pairs: &[(&str, JsValue)]) -> JsValue {
    let o = Object::new();
    for (k, v) in pairs {
        let _ = Reflect::set(&o, &(*k).into(), v);
    }
    o.into()
}

fn feed(s: &mut State) {
    if s.out.take_flushed() {
        s.generation = s.generation.wrapping_add(1);
        s.posted = 0;
        s.heard = 0;
        send_worklet(
            s,
            &obj(&[("t", "flush".into()), ("gen", s.generation.into())]),
            None,
        );
    }
    let paused = s.out.is_paused();
    if paused != s.paused {
        s.paused = paused;
        send_worklet(
            s,
            &obj(&[("t", "pause".into()), ("v", paused.into())]),
            None,
        );
    }
    let target = (AHEAD * f64::from(s.rate)) as u64;
    while s.posted.saturating_sub(s.heard) < target {
        let n = s.out.pull(&mut s.buf);
        if n == 0 {
            break;
        }
        let block = Float32Array::from(&s.buf[..n * 2]);
        let buffer = block.buffer();
        send_worklet(
            s,
            &obj(&[("t", "pcm".into()), ("data", block.into())]),
            Some(&buffer.into()),
        );
        s.posted += n as u64;
    }
    s.out.set_queued(s.posted.saturating_sub(s.heard));
}

fn start_downloads(s: &mut State) {
    // Fetch the two songs after the playing one early, so that the transition is gapless,
    // but one at a time: the next download starts when the song before it is in memory, so
    // the playing song gets the whole connection (several players may share it).
    let snap = s.engine.snapshot();
    if let Some(i) = snap.current_index {
        let mut before = snap.queue.get(i).map(|e| e.track.id.as_str());
        for e in snap.queue.iter().skip(i + 1).take(2) {
            if !before.is_some_and(|id| s.sources.is_loaded(id)) {
                break;
            }
            s.sources.want(&e.track.id);
            before = Some(&e.track.id);
        }
    }
    for (id, url) in s.sources.take_requests() {
        let sources = s.sources.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let result = download(&url).await;
            if let Err(e) = &result {
                post(&FromWorker::Error {
                    text: format!("download failed: {e}"),
                });
            }
            sources.finished(&id, result);
            tick();
        });
    }
}

async fn download(url: &str) -> Result<Vec<u8>, String> {
    let js = |e: JsValue| e.as_string().unwrap_or_else(|| format!("{e:?}"));
    let resp: Response = JsFuture::from(scope().fetch_with_str(url))
        .await
        .map_err(js)?
        .dyn_into()
        .map_err(js)?;
    if !resp.ok() {
        return Err(format!("HTTP {}", resp.status()));
    }
    let buf = JsFuture::from(resp.array_buffer().map_err(js)?)
        .await
        .map_err(js)?;
    let bytes = Uint8Array::new(&buf).to_vec();
    // Subsonic errors come back as a small JSON or XML body with status 200.
    if bytes.len() < 2048 && (bytes.starts_with(b"{") || bytes.starts_with(b"<?xml")) {
        return Err(String::from_utf8_lossy(&bytes).into_owned());
    }
    Ok(bytes)
}

fn report(s: &mut State) {
    let mut queue_changed = false;
    let mut important = false;
    for event in s.engine.events().try_iter() {
        match &event {
            Event::QueueChanged => queue_changed = true,
            Event::Position { .. } => {}
            _ => important = true,
        }
        if !matches!(event, Event::QueueChanged) {
            post(&FromWorker::Event { event });
        }
    }
    let snap = s.engine.snapshot();
    if queue_changed {
        post(&FromWorker::Queue {
            entries: snap.queue.to_vec(),
        });
    }
    let t = now_ms();
    let playing = matches!(snap.state, PlayState::Playing | PlayState::Buffering);
    if important || queue_changed || (playing && t - s.last_status > 250.0) {
        s.last_status = t;
        let (cached, loading) = s.sources.counts();
        let mut snapshot = snap;
        snapshot.queue = Arc::default();
        post(&FromWorker::Status {
            snapshot,
            cached,
            loading,
        });
    }
}
