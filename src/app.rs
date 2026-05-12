pub fn run_app(command: crate::cli::AppCommand) -> std::io::Result<()> {
    crate::tui::run_app(command)
}
