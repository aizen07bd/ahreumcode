use ratatui::layout::Alignment;
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

use crate::product;

use super::super::style;

const AHREUM_ROWS: [&str; 5] = [
    "  ▄▄▄▄   ▄▄                               ",
    "▄██▀▀██▄ ██                               ",
    "███  ███ ████▄ ████▄ ▄█▀█▄ ██ ██ ███▄███▄ ",
    "███▀▀███ ██ ██ ██ ▀▀ ██▄█▀ ██ ██ ██ ██ ██ ",
    "███  ███ ██ ██ ██    ▀█▄▄▄ ▀██▀█ ██ ██ ██ ",
];

const CODE_ROWS: [&str; 5] = [
    " ▄▄▄▄▄▄▄          ▄▄       ",
    "███▀▀▀▀▀          ██       ",
    "███      ▄███▄ ▄████ ▄█▀█▄ ",
    "███      ██ ██ ██ ██ ██▄█▀ ",
    "▀███████ ▀███▀ ▀████ ▀█▄▄▄ ",
];

pub fn lines() -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = AHREUM_ROWS
        .iter()
        .zip(CODE_ROWS.iter())
        .enumerate()
        .map(|(index, (ahreum, code))| {
            let ahreum_style = if index == AHREUM_ROWS.len() - 1 {
                style::cyan()
            } else {
                style::logo_ahreum()
            };
            let code_style = if index == CODE_ROWS.len() - 1 {
                style::cyan()
            } else {
                style::logo_code()
            };

            Line::from(vec![
                Span::styled((*ahreum).to_owned(), ahreum_style),
                Span::styled((*code).to_owned(), code_style),
            ])
            .alignment(Alignment::Center)
        })
        .collect();

    lines.push(
        Line::from(Span::styled(version_line(), style::muted())).alignment(Alignment::Center),
    );

    lines
}

fn version_line() -> String {
    let text_width = product::KOREAN_VERSION_LINE.width();
    let padding = width().saturating_sub(text_width);
    format!("{}{}", " ".repeat(padding), product::KOREAN_VERSION_LINE)
}

fn width() -> usize {
    AHREUM_ROWS[0].width() + CODE_ROWS[0].width()
}
