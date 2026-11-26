// SPDX-License-Identifier: GPL-3.0-only
//! Cover art of the playing song: cached on disk, drawn with the best protocol the terminal
//! supports (Kitty, Sixel, iTerm2 or half blocks).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use infrasonic_core::Session;
use ratatui::Frame;
use ratatui::layout::Rect;

/// Pixel size requested from the server.
const SIZE: u32 = 600;

/// A fetch to run in the background.
#[derive(Debug)]
pub struct CoverJob {
    id: String,
    cache_dir: Option<PathBuf>,
    decode: bool,
}

/// Result of a [`CoverJob`].
#[derive(Debug)]
pub struct CoverMsg {
    pub id: String,
    pub result: Result<CoverData, String>,
}

#[derive(Debug)]
pub struct CoverData {
    /// The image on disk, for the OS media controls.
    pub path: Option<PathBuf>,
    #[cfg(feature = "cover-art")]
    image: Option<image::DynamicImage>,
}

/// File name for a cover id (ids are opaque strings).
pub fn cache_file(dir: &Path, id: &str) -> PathBuf {
    let safe: String = id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .take(100)
        .collect();
    dir.join(format!("{safe}-{SIZE}.img"))
}

impl CoverJob {
    pub async fn run(self, s: Arc<Session>) -> CoverMsg {
        let result = self.fetch(&s).await;
        CoverMsg {
            id: self.id,
            result,
        }
    }

    async fn fetch(&self, s: &Session) -> Result<CoverData, String> {
        let path = self.cache_dir.as_ref().map(|d| cache_file(d, &self.id));
        let cached = match &path {
            Some(p) => tokio::fs::read(p).await.ok(),
            None => None,
        };
        let bytes = match cached {
            Some(b) => b,
            None => {
                let b = s
                    .client()
                    .get_cover_art(&self.id, Some(SIZE))
                    .await
                    .map_err(|e| e.to_string())?;
                if let Some(p) = &path {
                    if let Some(dir) = p.parent() {
                        let _ = tokio::fs::create_dir_all(dir).await;
                    }
                    if let Err(e) = tokio::fs::write(p, &b).await {
                        tracing::debug!("caching cover failed: {e}");
                    }
                }
                b
            }
        };
        let written = path.filter(|p| p.exists());
        #[cfg(feature = "cover-art")]
        {
            let image = if self.decode {
                tokio::task::spawn_blocking(move || image::load_from_memory(&bytes))
                    .await
                    .map_err(|e| e.to_string())?
                    .map(Some)
                    .map_err(|e| format!("cover image: {e}"))?
            } else {
                None
            };
            Ok(CoverData {
                path: written,
                image,
            })
        }
        #[cfg(not(feature = "cover-art"))]
        {
            let _ = (bytes, self.decode);
            Ok(CoverData { path: written })
        }
    }
}

/// Image protocol state for the current cover.
pub struct Cover {
    enabled: bool,
    cache_dir: Option<PathBuf>,
    wanted: Option<String>,
    /// Cover file of the playing song, once available.
    pub file: Option<(String, PathBuf)>,
    #[cfg(feature = "cover-art")]
    picker: Option<ratatui_image::picker::Picker>,
    #[cfg(feature = "cover-art")]
    protocol: Option<ratatui_image::protocol::StatefulProtocol>,
}

impl std::fmt::Debug for Cover {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Cover")
            .field("enabled", &self.enabled)
            .field("wanted", &self.wanted)
            .finish_non_exhaustive()
    }
}

impl Cover {
    /// No images, no cache (tests).
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            cache_dir: None,
            wanted: None,
            file: None,
            #[cfg(feature = "cover-art")]
            picker: None,
            #[cfg(feature = "cover-art")]
            protocol: None,
        }
    }

    /// Query the terminal for its graphics support. Call in raw mode, before reading events.
    pub fn new(show: bool, cache_dir: Option<PathBuf>) -> Self {
        let mut c = Self::disabled();
        c.enabled = true;
        c.cache_dir = cache_dir;
        #[cfg(feature = "cover-art")]
        if show {
            c.picker = Some(
                ratatui_image::picker::Picker::from_query_stdio()
                    .unwrap_or_else(|_| ratatui_image::picker::Picker::halfblocks()),
            );
        }
        #[cfg(not(feature = "cover-art"))]
        let _ = show;
        c
    }

    /// The playing song's cover changed. Returns a job if it has to be fetched.
    pub fn want(&mut self, id: Option<String>) -> Option<CoverJob> {
        if !self.enabled || id == self.wanted {
            return None;
        }
        self.wanted = id.clone();
        self.file = None;
        #[cfg(feature = "cover-art")]
        {
            self.protocol = None;
        }
        Some(CoverJob {
            id: id?,
            cache_dir: self.cache_dir.clone(),
            #[cfg(feature = "cover-art")]
            decode: self.picker.is_some(),
            #[cfg(not(feature = "cover-art"))]
            decode: false,
        })
    }

    pub fn loaded(&mut self, msg: CoverMsg) {
        if self.wanted.as_deref() != Some(msg.id.as_str()) {
            return;
        }
        match msg.result {
            Ok(data) => {
                self.file = data.path.map(|p| (msg.id.clone(), p));
                #[cfg(feature = "cover-art")]
                if let (Some(p), Some(img)) = (&self.picker, data.image) {
                    self.protocol = Some(p.new_resize_protocol(img));
                }
            }
            Err(e) => tracing::debug!("cover {}: {e}", msg.id),
        }
    }

    pub fn draw(&mut self, f: &mut Frame, area: Rect) {
        if area.height == 0 {
            return;
        }
        #[cfg(feature = "cover-art")]
        if let Some(p) = &mut self.protocol {
            f.render_stateful_widget(ratatui_image::StatefulImage::default(), area, p);
            return;
        }
        let _ = f;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_names_are_safe() {
        let p = cache_file(Path::new("/c"), "../al-1/x y");
        assert_eq!(p, Path::new("/c/___al-1_x_y-600.img"));
    }
}
