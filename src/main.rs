mod app;
mod cli;
mod config;
mod llm;
mod logging;
mod product;
mod tui;

fn main() -> std::io::Result<()> {
    app::run_app(cli::AppCommand::parse(std::env::args().skip(1))?)
}
