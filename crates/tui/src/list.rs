// SPDX-License-Identifier: GPL-3.0-only
//! A selectable list with a fuzzy filter, shared by all pages.

use ratatui::layout::Rect;

/// Case-insensitive subsequence match. Returns a score (lower is better) or `None`.
pub fn fuzzy_score(needle: &str, hay: &str) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    let hay: Vec<char> = hay.chars().flat_map(char::to_lowercase).collect();
    let mut pos = 0;
    let mut first = None;
    let mut gaps = 0;
    let mut last = None;
    for n in needle.chars().flat_map(char::to_lowercase) {
        let found = hay[pos..].iter().position(|h| *h == n)? + pos;
        first.get_or_insert(found);
        if let Some(l) = last {
            gaps += found - l - 1;
        }
        last = Some(found);
        pos = found + 1;
    }
    Some(first.unwrap_or(0) + gaps * 2)
}

/// Items plus selection, scroll offset and filter.
#[derive(Debug, Clone)]
pub struct ListView<T> {
    items: Vec<T>,
    /// Indexes into `items` that pass the filter, in display order.
    visible: Vec<usize>,
    /// Position in `visible`.
    selected: usize,
    pub offset: usize,
    pub filter: String,
    /// The filter input is open.
    pub filtering: bool,
    /// Area of the last render, for mouse handling.
    pub area: Rect,
    /// Header rows drawn above the items in the last render.
    pub header_rows: u16,
}

impl<T> Default for ListView<T> {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            visible: Vec::new(),
            selected: 0,
            offset: 0,
            filter: String::new(),
            filtering: false,
            area: Rect::default(),
            header_rows: 0,
        }
    }
}

/// Something that can be filtered by text.
pub trait Filterable {
    fn filter_text(&self) -> String;
}

impl<T: Filterable> ListView<T> {
    pub fn new(items: Vec<T>) -> Self {
        let mut l = Self::default();
        l.set_items(items);
        l
    }

    /// Replace the items, keeping the filter and (clamped) selection.
    pub fn set_items(&mut self, items: Vec<T>) {
        self.items = items;
        self.refilter();
    }

    pub fn set_filter(&mut self, f: String) {
        self.filter = f;
        self.refilter();
        self.selected = 0;
        self.offset = 0;
    }

    pub fn clear_filter(&mut self) {
        let keep = self.selected_index();
        self.filter.clear();
        self.filtering = false;
        self.refilter();
        if let Some(i) = keep {
            self.select_index(i);
        }
    }

    fn refilter(&mut self) {
        if self.filter.is_empty() {
            self.visible = (0..self.items.len()).collect();
        } else {
            let mut scored: Vec<(usize, usize)> = self
                .items
                .iter()
                .enumerate()
                .filter_map(|(i, it)| fuzzy_score(&self.filter, &it.filter_text()).map(|s| (s, i)))
                .collect();
            scored.sort();
            self.visible = scored.into_iter().map(|(_, i)| i).collect();
        }
        self.selected = self.selected.min(self.visible.len().saturating_sub(1));
    }
}

impl<T> ListView<T> {
    pub fn items(&self) -> &[T] {
        &self.items
    }
    pub fn items_mut(&mut self) -> &mut [T] {
        &mut self.items
    }
    pub fn has_filter_applied(&self) -> bool {
        !self.filter.is_empty()
    }
    pub fn len(&self) -> usize {
        self.visible.len()
    }
    pub fn is_empty(&self) -> bool {
        self.visible.is_empty()
    }
    /// Visible items with their item index.
    pub fn visible(&self) -> impl Iterator<Item = (usize, &T)> {
        self.visible.iter().map(|&i| (i, &self.items[i]))
    }
    /// Visible items in display order.
    pub fn visible_items(&self) -> Vec<&T> {
        self.visible.iter().map(|&i| &self.items[i]).collect()
    }
    /// Position of the selection among the visible items.
    pub fn cursor(&self) -> usize {
        self.selected
    }
    /// Index into `items` of the selected entry.
    pub fn selected_index(&self) -> Option<usize> {
        self.visible.get(self.selected).copied()
    }
    pub fn selected(&self) -> Option<&T> {
        self.selected_index().map(|i| &self.items[i])
    }
    pub fn selected_mut(&mut self) -> Option<&mut T> {
        self.selected_index().map(|i| &mut self.items[i])
    }
    /// Select by item index (if visible).
    pub fn select_index(&mut self, item: usize) {
        if let Some(p) = self.visible.iter().position(|&i| i == item) {
            self.selected = p;
        }
    }
    pub fn select_where(&mut self, mut f: impl FnMut(&T) -> bool) {
        if let Some(p) = self.visible.iter().position(|&i| f(&self.items[i])) {
            self.selected = p;
        }
    }
    pub fn set_cursor(&mut self, pos: usize) {
        self.selected = pos.min(self.visible.len().saturating_sub(1));
    }
    pub fn move_by(&mut self, delta: isize) {
        if self.visible.is_empty() {
            return;
        }
        let max = self.visible.len() as isize - 1;
        self.selected = (self.selected as isize + delta).clamp(0, max) as usize;
    }
    pub fn top(&mut self) {
        self.selected = 0;
    }
    pub fn bottom(&mut self) {
        self.selected = self.visible.len().saturating_sub(1);
    }
    /// Rows available for items in the last render.
    pub fn page(&self) -> isize {
        (self.area.height.saturating_sub(2 + self.header_rows)).max(1) as isize
    }
    /// Keep the selection on screen for a viewport of `height` rows.
    pub fn scroll_to_selection(&mut self, height: usize) {
        if height == 0 {
            return;
        }
        if self.selected < self.offset {
            self.offset = self.selected;
        } else if self.selected >= self.offset + height {
            self.offset = self.selected + 1 - height;
        }
        let max_offset = self.visible.len().saturating_sub(height);
        self.offset = self.offset.min(max_offset);
    }
    /// The visible row at screen row `row` of the list body, if any.
    pub fn row_at(&self, row: u16, body_top: u16) -> Option<usize> {
        let r = row.checked_sub(body_top)? as usize + self.offset;
        (r < self.visible.len()).then_some(r)
    }
}

/// Object-safe list operations, so that key handling works on whatever list has focus.
pub trait ListLike {
    fn move_by(&mut self, delta: isize);
    fn top(&mut self);
    fn bottom(&mut self);
    fn page(&self) -> isize;
    fn filtering(&self) -> bool;
    fn has_filter(&self) -> bool;
    fn begin_filter(&mut self);
    fn filter_push(&mut self, c: char);
    fn filter_pop(&mut self);
    /// Close the filter input; `keep` keeps the filter applied.
    fn end_filter(&mut self, keep: bool);
    fn select_row(&mut self, row: usize);
    fn row_at(&self, row: u16) -> Option<usize>;
    fn area(&self) -> Rect;
}

impl<T: Filterable> ListLike for ListView<T> {
    fn move_by(&mut self, delta: isize) {
        ListView::move_by(self, delta);
    }
    fn top(&mut self) {
        ListView::top(self);
    }
    fn bottom(&mut self) {
        ListView::bottom(self);
    }
    fn page(&self) -> isize {
        ListView::page(self)
    }
    fn filtering(&self) -> bool {
        self.filtering
    }
    fn has_filter(&self) -> bool {
        !self.filter.is_empty()
    }
    fn begin_filter(&mut self) {
        self.filtering = true;
    }
    fn filter_push(&mut self, c: char) {
        let mut f = std::mem::take(&mut self.filter);
        f.push(c);
        self.set_filter(f);
    }
    fn filter_pop(&mut self) {
        let mut f = std::mem::take(&mut self.filter);
        f.pop();
        self.set_filter(f);
    }
    fn end_filter(&mut self, keep: bool) {
        self.filtering = false;
        if !keep || self.filter.is_empty() {
            self.clear_filter();
        }
    }
    fn select_row(&mut self, row: usize) {
        self.set_cursor(row);
    }
    fn row_at(&self, row: u16) -> Option<usize> {
        // Lists are drawn inside a border, optionally with a header row.
        ListView::row_at(self, row, self.area.y + 1 + self.header_rows)
    }
    fn area(&self) -> Rect {
        self.area
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    impl Filterable for &'static str {
        fn filter_text(&self) -> String {
            (*self).to_string()
        }
    }

    #[test]
    fn fuzzy() {
        assert!(fuzzy_score("bnc", "Black Noise Collective").is_some());
        assert!(fuzzy_score("xyz", "Black Noise Collective").is_none());
        assert!(fuzzy_score("black", "Black") < fuzzy_score("black", "a Bl_ack"));
        assert_eq!(fuzzy_score("", "x"), Some(0));
    }

    #[test]
    fn filter_keeps_selection() {
        let mut l = ListView::new(vec!["alpha", "beta", "gamma", "delta"]);
        l.set_cursor(2);
        l.set_filter("ta".into());
        assert_eq!(l.visible_items(), [&"beta", &"delta"]);
        l.move_by(1);
        assert_eq!(l.selected(), Some(&"delta"));
        l.clear_filter();
        assert_eq!(l.selected(), Some(&"delta"));
        assert_eq!(l.len(), 4);
    }

    #[test]
    fn scrolling() {
        let mut l = ListView::new((0..100).map(|_| "x").collect());
        l.set_cursor(50);
        l.scroll_to_selection(10);
        assert_eq!(l.offset, 41);
        l.top();
        l.scroll_to_selection(10);
        assert_eq!(l.offset, 0);
        assert_eq!(l.row_at(5, 2), Some(3));
        l.set_items(vec!["a"]);
        assert_eq!(l.cursor(), 0);
    }
}
