//! `<leader>p` file picker modal (`InputMode::FilePicker`). Laid out like the
//! `:theme` picker (`ui/theme_picker.rs`); this module only renders the
//! current `FilePickerState`.

use crate::ui::app_layout::centered_rect;
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph},
};

use crate::app::App;
use crate::ui::styles;
use crate::ui::text_utils::prefix_within_width;
use unicode_width::UnicodeWidthStr;

/// Diff-area width below which the popup spans the whole diff.
const NARROW_WIDTH: u16 = 80;

/// Like `truncate_path_smart`, but also returns the character offset into
/// `path` where the kept content starts in the original string, and the
/// offset within the truncated string where that content begins.
///
/// This lets callers map match positions (character offsets in the full
/// path) to positions in the truncated string: for a match at position `p`,
/// if `p >= original_offset`, its position in the truncated string is
/// `truncated_content_offset + (p - original_offset)`; otherwise it was
/// truncated away.
fn truncate_path_with_offset(path: &str, max_width: usize) -> (String, usize, usize) {
    if path.width() <= max_width {
        return (path.to_string(), 0, 0);
    }

    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    if segments.is_empty() {
        return (path.to_string(), 0, 0);
    }

    // "…/tail" with n tail segments
    for n in (1..segments.len()).rev() {
        let tail = segments[segments.len() - n..].join("/");
        let candidate = format!("…/{tail}");
        if candidate.width() <= max_width {
            let dropped = segments[..segments.len() - n].join("/");
            let original_offset = dropped.chars().count() + 1; // +1 for the "/"
            return (candidate, original_offset, 2); // content starts after "…"
        }
    }

    // Just basename
    let basename = segments.last().unwrap();
    let basename_byte_offset = path.rfind(basename).unwrap_or(path.len() - basename.len());
    let original_offset = path[..basename_byte_offset].chars().count();

    if basename.width() <= max_width {
        return (basename.to_string(), original_offset, 0);
    }

    // Basename too wide: keep leading chars + …
    let kept = max_width.saturating_sub(1);
    let mut s = prefix_within_width(basename, kept).to_string();
    s.push('…');
    return (s, original_offset, 0);
}

pub fn render_file_picker(frame: &mut Frame, app: &mut App) {
    let theme = &app.theme;
    // Wider than the theme picker: paths need the room. In a narrow pane (the
    // case the picker exists for, with the file list hidden) take the whole
    // diff width rather than a share of it.
    let diff_area = app.diff_area.unwrap_or(frame.area());
    let percent_x = if diff_area.width < NARROW_WIDTH {
        100
    } else {
        70
    };
    let area = centered_rect(percent_x, 60, diff_area);
    frame.render_widget(Clear, area);

    let filtered = app.file_picker.filtered_indices();
    let title = format!(
        " Go to File ({}/{}) ",
        filtered.len(),
        app.file_picker.candidates.len()
    );
    let block = Block::default()
        .title(title)
        .title_alignment(Alignment::Center)
        .borders(Borders::ALL)
        .style(styles::popup_style(theme))
        .border_style(styles::border_style(theme, true));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let filtering = app.file_picker_filtering();
    let filter_height: u16 = if filtering { 1 } else { 0 };
    let [filter_area, list_area, footer_area] = Layout::vertical([
        Constraint::Length(filter_height),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .areas(inner);

    if filter_height > 0 {
        let text = app.file_picker.draft.as_deref().unwrap_or("");
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("/", Style::default().fg(theme.fg_secondary)),
                Span::styled(text, Style::default().fg(theme.fg_primary)),
            ])),
            filter_area,
        );
    }

    if filtered.is_empty() {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "No files match",
                styles::dim_style(theme),
            ))),
            list_area,
        );
    } else {
        // Status glyph + space take two cells; long paths lose leading
        // directories (`…/github/review_threads.rs`) so the basename stays.
        let path_width = usize::from(list_area.width).saturating_sub(2).max(1);
        let items: Vec<ListItem> = filtered
            .iter()
            .map(|&(candidate_idx, ref positions)| {
                let (file_idx, path) = &app.file_picker.candidates[candidate_idx];
                let status = app
                    .diff_files
                    .get(*file_idx)
                    .map(|file| file.status.as_char())
                    .unwrap_or(' ');
                let (truncated, original_offset, truncated_content_offset) =
                    truncate_path_with_offset(path, path_width);

                let mut all_spans: Vec<Span> = vec![Span::styled(
                    format!("{status} "),
                    styles::file_status_style(theme, status),
                )];

                if positions.is_empty() {
                    // No draft: show path unmarked
                    all_spans.push(Span::raw(truncated));
                } else {
                    // Map match positions from the full path to the truncated string
                    let match_positions: Vec<usize> = positions
                        .iter()
                        .filter_map(|&p| {
                            if p < original_offset {
                                None
                            } else {
                                Some(truncated_content_offset + (p - original_offset))
                            }
                        })
                        .collect();

                    let mut buf = String::new();
                    let mut in_match = false;
                    let mut match_idx = 0;
                    for (i, ch) in truncated.chars().enumerate() {
                        let is_match =
                            match_idx < match_positions.len() && match_positions[match_idx] == i;
                        if is_match {
                            match_idx += 1;
                        }
                        if is_match != in_match {
                            if !buf.is_empty() {
                                all_spans.push(Span::styled(
                                    buf.clone(),
                                    if in_match {
                                        Style::default()
                                            .fg(theme.fg_primary)
                                            .add_modifier(Modifier::BOLD)
                                    } else {
                                        Style::default().fg(theme.fg_secondary)
                                    },
                                ));
                                buf.clear();
                            }
                            in_match = is_match;
                        }
                        buf.push(ch);
                    }
                    if !buf.is_empty() {
                        all_spans.push(Span::styled(
                            buf.clone(),
                            if in_match {
                                Style::default()
                                    .fg(theme.fg_primary)
                                    .add_modifier(Modifier::BOLD)
                            } else {
                                Style::default().fg(theme.fg_secondary)
                            },
                        ));
                    }
                }

                ListItem::new(Line::from(all_spans))
            })
            .collect();
        let list = List::new(items)
            .style(styles::panel_style(theme))
            .highlight_style(styles::selected_style(theme));
        frame.render_stateful_widget(list, list_area, &mut app.file_picker.list_state);
    }

    let footer = if filtering {
        "type to filter \u{00b7} \u{2191}/\u{2193} move \u{00b7} \u{21b5} open \u{00b7} esc cancel filter"
    } else {
        "j/k move \u{00b7} \u{21b5} open \u{00b7} / filter \u{00b7} esc cancel"
    };
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            footer,
            Style::default()
                .fg(theme.fg_secondary)
                .add_modifier(Modifier::DIM),
        ))),
        footer_area,
    );
}
