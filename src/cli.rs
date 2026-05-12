use std::io;

#[derive(Clone, Copy)]
pub struct AppCommand {
    pub scene: SceneCommand,
    pub run_mode: RunMode,
}

#[derive(Clone, Copy)]
pub enum SceneCommand {
    Intro,
    Main,
    Epilogue,
}

#[derive(Clone, Copy)]
pub enum RunMode {
    Normal,
    Dev,
    Smoke,
}

impl RunMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Dev => "dev",
            Self::Smoke => "smoke",
        }
    }
}

impl AppCommand {
    pub fn parse(args: impl Iterator<Item = String>) -> io::Result<Self> {
        let mut command = Self {
            scene: SceneCommand::Intro,
            run_mode: RunMode::Normal,
        };

        let mut args = args.peekable();
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--dev" => command.run_mode = RunMode::Dev,
                "--smoke" => command.run_mode = RunMode::Smoke,
                "--scene" => {
                    let Some(scene) = args.next() else {
                        return Err(invalid_input("--scene requires a value"));
                    };
                    command.scene = parse_scene(&scene)?;
                }
                "--help" | "-h" => {
                    print_help();
                    command.run_mode = RunMode::Smoke;
                }
                unknown => return Err(invalid_input(&format!("unknown argument: {unknown}"))),
            }
        }

        Ok(command)
    }
}

fn parse_scene(value: &str) -> io::Result<SceneCommand> {
    match value {
        "intro" => Ok(SceneCommand::Intro),
        "main" => Ok(SceneCommand::Main),
        "epilogue" => Ok(SceneCommand::Epilogue),
        _ => Err(invalid_input(&format!("unknown scene: {value}"))),
    }
}

fn print_help() {
    println!("AhreumCode");
    println!("  cargo run");
    println!("  cargo run -- --scene intro");
    println!("  cargo run -- --scene main");
    println!("  cargo run -- --scene main --smoke");
    println!("  cargo run -- --scene epilogue --smoke");
    println!("  cargo run -- --dev --scene intro");
    println!("  cargo run -- --scene intro --smoke");
}

fn invalid_input(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.to_owned())
}
