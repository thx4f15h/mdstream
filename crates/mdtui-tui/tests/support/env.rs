use std::ffi::OsStr;
use std::process::Command;

use portable_pty::CommandBuilder;

pub const FIXED_ENV: [(&str, &str); 5] = [
    ("TERM", "xterm-256color"),
    ("COLORTERM", "truecolor"),
    ("LANG", "C.UTF-8"),
    ("COLUMNS", "120"),
    ("LINES", "40"),
];

pub struct FixedEnv;

impl FixedEnv {
    pub fn configure_std(command: &mut Command) {
        command.env_clear();
        for (key, value) in FIXED_ENV {
            command.env(key, value);
        }
        command.env_remove("NO_COLOR");
    }

    pub fn configure_pty(command: &mut CommandBuilder) {
        command.env_clear();
        for (key, value) in FIXED_ENV {
            command.env(key, value);
        }
        command.env_remove("NO_COLOR");
    }
}

pub fn fixed_std_command(program: impl AsRef<OsStr>) -> Command {
    let mut command = Command::new(program);
    FixedEnv::configure_std(&mut command);
    command
}

pub fn fixed_pty_command(program: impl AsRef<OsStr>) -> CommandBuilder {
    let mut command = CommandBuilder::new(program);
    FixedEnv::configure_pty(&mut command);
    command
}
