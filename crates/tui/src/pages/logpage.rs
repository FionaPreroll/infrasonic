// SPDX-License-Identifier: GPL-3.0-only
//! Page 5: recent log messages.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use tracing::Level;

use crate::list::{Filterable, ListView};
use crate::logging::{LogBuffer, LogLine};
use crate::t;
use crate::theme;
use crate::ui::{self, Column};

impl Filterable for LogLine {
    fn filter_text(&self) -> String {
        format!("{} {} {}", self.level, self.target, self.message)
    }
}

#[derive(Debug)]
pub struct LogPage {
    buffer: LogBuffer,
    generation: u64,
    pub list: ListView<LogLine>,
}

impl LogPage {
    pub fn new(buffer: LogBuffer) -> Self {
        Self {
            buffer,
            generation: u64::MAX,
            list: ListView::default(),
        }
    }

    /// Pull new lines; stays at the end if it was there.
    pub fn sync(&mut self) {
        let g = self.buffer.generation();
        if g == self.generation {
            return;
        }
        self.generation = g;
        let at_end = self.list.cursor() + 1 >= self.list.len();
        self.list.set_items(self.buffer.lines());
        if at_end {
            self.list.bottom();
        }
    }
}

pub fn draw(page: &mut LogPage, f: &mut Frame, area: Rect) {
    page.sync();
    ui::draw_list(
        f,
        area,
        &mut page.list,
        &Column {
            title: t!("log.title").into(),
            focused: true,
            loading: false,
            empty: t!("log.empty"),
        },
        |l| {
            let style = match l.level {
                Level::ERROR => theme::error(),
                Level::WARN => theme::warn(),
                Level::INFO => ratatui::style::Style::default(),
                _ => theme::dim(),
            };
            Line::from(vec![
                Span::styled(format!("{} ", l.time), theme::dim()),
                Span::styled(format!("{:5} ", l.level), style),
                Span::styled(l.message.clone(), style),
            ])
        },
    );
}
