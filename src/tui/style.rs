use ratatui::style::{Color, Modifier, Style};

pub fn logo_ahreum() -> Style {
    Style::default()
        .fg(Color::Rgb(132, 132, 132))
        .add_modifier(Modifier::BOLD)
}

pub fn logo_code() -> Style {
    Style::default()
        .fg(Color::Rgb(200, 200, 200))
        .add_modifier(Modifier::BOLD)
}

pub fn cyan() -> Style {
    Style::default().fg(Color::Cyan)
}

pub fn muted() -> Style {
    Style::default().fg(Color::Rgb(135, 135, 135))
}

pub fn panel() -> Style {
    Style::default().fg(Color::Rgb(210, 210, 210))
}

pub fn panel_bold() -> Style {
    panel().add_modifier(Modifier::BOLD)
}

pub fn persona_lead() -> Style {
    cyan().add_modifier(Modifier::BOLD)
}

pub fn persona_speaker() -> Style {
    muted().add_modifier(Modifier::BOLD)
}

pub fn persona_time() -> Style {
    Style::default().fg(Color::Rgb(110, 110, 110))
}

pub fn persona_accent() -> Style {
    cyan()
}

pub fn prompt_background() -> Style {
    Style::default().bg(Color::Rgb(38, 38, 38))
}

pub fn statusline() -> Style {
    Style::default()
        .fg(Color::Rgb(170, 170, 170))
        .bg(Color::Rgb(24, 24, 24))
}
