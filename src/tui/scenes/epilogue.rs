use std::io::{self, Write};

use crossterm::style::{style, Attribute, Color, Stylize};
use unicode_width::UnicodeWidthStr;

use crate::product;

use super::super::state::EpilogueSummary;

const CONTENT_WIDTH: usize = 66;

#[derive(Clone, Copy)]
enum PrintStyle {
    Muted,
    Panel,
    Bold,
    Cyan,
}

struct Segment<'a> {
    text: &'a str,
    style: PrintStyle,
}

impl<'a> Segment<'a> {
    fn muted(text: &'a str) -> Self {
        Self {
            text,
            style: PrintStyle::Muted,
        }
    }

    fn panel(text: &'a str) -> Self {
        Self {
            text,
            style: PrintStyle::Panel,
        }
    }

    fn bold(text: &'a str) -> Self {
        Self {
            text,
            style: PrintStyle::Bold,
        }
    }

    fn cyan(text: &'a str) -> Self {
        Self {
            text,
            style: PrintStyle::Cyan,
        }
    }
}

pub fn print_epilogue(summary: &EpilogueSummary) -> io::Result<()> {
    let mut stdout = io::stdout();
    write_epilogue(&mut stdout, summary)
}

pub fn write_epilogue(writer: &mut impl Write, summary: &EpilogueSummary) -> io::Result<()> {
    writeln!(writer)?;
    writeln!(writer, "╭{}╮", "─".repeat(CONTENT_WIDTH))?;
    write_row(
        writer,
        &[Segment::muted(">_ "), Segment::bold(product::APP_NAME)],
    )?;
    write_row(writer, &[Segment::muted(product::KOREAN_VERSION_LINE)])?;
    write_row(writer, &[])?;
    write_row(
        writer,
        &[
            Segment::muted("workspace: "),
            Segment::panel(summary.workspace.as_str()),
        ],
    )?;
    write_row(
        writer,
        &[
            Segment::muted("model: "),
            Segment::panel(summary.model),
            Segment::panel("   "),
            Segment::muted("mode: "),
            Segment::panel(summary.mode),
        ],
    )?;
    write_row(
        writer,
        &[Segment::muted("session: "), Segment::panel(summary.session)],
    )?;
    let tools_executed = summary.tools_executed.to_string();
    let tools_failed = summary.tools_failed.to_string();
    write_row(
        writer,
        &[
            Segment::muted("tools: "),
            Segment::panel(&tools_executed),
            Segment::panel(" executed   "),
            Segment::panel(&tools_failed),
            Segment::panel(" failed"),
        ],
    )?;
    write_row(writer, &[Segment::cyan(summary.closing_message)])?;
    writeln!(writer, "╰{}╯", "─".repeat(CONTENT_WIDTH))?;
    writeln!(writer)?;
    write_tip(writer)?;
    writer.flush()
}

fn write_row(writer: &mut impl Write, segments: &[Segment<'_>]) -> io::Result<()> {
    let content_width = visible_width(segments);
    let padding = CONTENT_WIDTH.saturating_sub(content_width);

    write!(writer, "│  ")?;
    for segment in segments {
        write_segment(writer, segment)?;
    }
    write!(writer, "{}│", " ".repeat(padding))?;
    writeln!(writer)
}

fn write_tip(writer: &mut impl Write) -> io::Result<()> {
    write_styled(writer, product::EPILOGUE_TIP_PREFIX, PrintStyle::Muted)?;
    write_styled(writer, product::EPILOGUE_TIP_COMMAND, PrintStyle::Bold)?;
    write_styled(writer, product::EPILOGUE_TIP_TEXT, PrintStyle::Panel)?;
    write_styled(
        writer,
        product::EPILOGUE_TIP_SESSIONS_COMMAND,
        PrintStyle::Bold,
    )?;
    write_styled(writer, product::EPILOGUE_TIP_SUFFIX, PrintStyle::Panel)?;
    writeln!(writer)
}

fn visible_width(segments: &[Segment<'_>]) -> usize {
    2 + segments
        .iter()
        .map(|segment| segment.text.width())
        .sum::<usize>()
}

fn write_segment(writer: &mut impl Write, segment: &Segment<'_>) -> io::Result<()> {
    write_styled(writer, segment.text, segment.style)
}

fn write_styled(writer: &mut impl Write, text: &str, print_style: PrintStyle) -> io::Result<()> {
    match print_style {
        PrintStyle::Muted => write!(writer, "{}", style(text).with(Color::DarkGrey)),
        PrintStyle::Panel => write!(writer, "{}", style(text).with(Color::Grey)),
        PrintStyle::Bold => write!(
            writer,
            "{}",
            style(text).with(Color::White).attribute(Attribute::Bold)
        ),
        PrintStyle::Cyan => write!(writer, "{}", style(text).with(Color::Cyan)),
    }
}
