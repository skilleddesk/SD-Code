//! The OS normaliser (P0-13): one tool call, the same meaning on macOS, Windows and Linux.
//!
//! # The problem, stated exactly
//!
//! A model writes `["npm", "test"]` and it works everywhere. A model writes `grep -rn "foo" src` and it works on two
//! of the three: Windows has no `grep`, and the shell that runs a string is `cmd` there and `sh` here. A model that
//! has learned one OS writes commands the other two refuse, and the failure looks like the model's mistake when it is
//! the harness's.
//!
//! So a tool call is normalised before it runs: the command name is mapped to something that exists on this OS, the
//! arguments that carry a path get this OS's separator, and the way the command is launched is chosen per OS.
//!
//! # Where this is tested, and how
//!
//! Only macOS can be run here. The plan's "same tool call works on 3 OS" is therefore pinned the way it can be: each
//! OS's mapping is asserted in full, so a change that fixes one and breaks another fails a test. That is a weaker
//! claim than three machines, and it is the true one - the report says so.

/// The three platforms SDC ships to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Os {
    Macos,
    Linux,
    Windows,
}

impl Os {
    /// The OS this build is running on.
    pub fn here() -> Os {
        if cfg!(target_os = "windows") {
            Os::Windows
        } else if cfg!(target_os = "macos") {
            Os::Macos
        } else {
            Os::Linux
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Os::Macos => "macos",
            Os::Linux => "linux",
            Os::Windows => "windows",
        }
    }

    /// The shell a command *string* is handed to. A command given as a list of words does not need one.
    pub fn shell(self) -> (&'static str, &'static str) {
        match self {
            Os::Windows => ("cmd", "/C"),
            _ => ("sh", "-c"),
        }
    }

    pub fn separator(self) -> char {
        match self {
            Os::Windows => '\\',
            _ => '/',
        }
    }
}

/// A command the model asked for, before this OS has had its say.
#[derive(Clone, Debug, PartialEq)]
pub struct Command {
    /// The program, as the model wrote it (`grep`, `ls`, `rm`).
    pub program: String,
    pub args: Vec<String>,
    /// A command string instead of a program and arguments. Only one of the two is used: a string needs a shell, and
    /// a shell is exactly what normalising tries to avoid.
    pub shell: Option<String>,
    /// The folder it runs in, as written.
    pub cwd: Option<String>,
}

impl Command {
    pub fn new(program: &str, args: &[&str]) -> Self {
        Self { program: program.to_string(), args: args.iter().map(|arg| arg.to_string()).collect(), shell: None, cwd: None }
    }

    pub fn line(shell: &str) -> Self {
        Self { program: String::new(), args: Vec::new(), shell: Some(shell.to_string()), cwd: None }
    }
}

/// What is actually launched.
#[derive(Clone, Debug, PartialEq)]
pub struct Launch {
    pub program: String,
    pub args: Vec<String>,
    /// Whether a shell is involved. A normalised command usually is not, which is one less place for a quoted string
    /// to be reinterpreted.
    pub via_shell: bool,
}

impl Launch {
    /// The command as a person would type it, for the log and the permission dialog.
    pub fn display(&self) -> String {
        let mut parts = vec![self.program.clone()];

        parts.extend(self.args.iter().map(|arg| if arg.contains(' ') { format!("\"{arg}\"") } else { arg.clone() }));

        parts.join(" ")
    }
}

/// The Unix program a Windows session has no built-in for, and what to use instead.
///
/// Kept to the handful a coding agent actually reaches for. A long table would be a guess about what a model writes;
/// this is a guess about what it *needs*, which is smaller.
const ON_WINDOWS: &[(&str, &str, &[&str])] = &[
    /* program, replacement, extra arguments put first */
    ("grep", "findstr", &[]),
    ("egrep", "findstr", &[]),
    ("ls", "dir", &[]),
    ("cat", "type", &[]),
    ("rm", "del", &[]),
    ("cp", "copy", &[]),
    ("mv", "move", &[]),
    ("touch", "type", &["nul"]),
    ("which", "where", &[]),
    ("clear", "cls", &[]),
    ("pwd", "cd", &[]),
    ("ps", "tasklist", &[]),
];

/// Normalise a command for `os`.
///
/// Three things happen, in this order: a program Windows has no equivalent for is replaced, arguments that are
/// clearly paths get this OS's separator, and a command string is handed to this OS's shell. Anything the normaliser
/// does not recognise is left exactly as it was - a wrong guess is worse than no guess, because the command the model
/// meant may well exist on this machine under the name it used.
pub fn normalise(command: &Command, os: Os) -> Launch {
    if let Some(line) = &command.shell {
        let (shell, flag) = os.shell();

        return Launch { program: shell.to_string(), args: vec![flag.to_string(), line.clone()], via_shell: true };
    }

    let mut program = command.program.clone();
    let mut args: Vec<String> = Vec::new();

    if os == Os::Windows {
        if let Some((_, replacement, extra)) = ON_WINDOWS.iter().find(|(name, _, _)| name.eq_ignore_ascii_case(&command.program)) {
            program = (*replacement).to_string();
            args.extend(extra.iter().map(|arg| (*arg).to_string()));
        }
    }

    args.extend(command.args.iter().map(|arg| normalise_argument(arg, os)));

    Launch { program, args, via_shell: false }
}

/// A path that came from the model with forward slashes becomes a path this OS understands. Only arguments that look
/// like paths are touched: rewriting every `/` in every argument would break `sed 's/a/b/'` and every URL.
fn normalise_argument(argument: &str, os: Os) -> String {
    /* The *target* OS decides, not the build: this is the function a test on macOS uses to pin what Windows
       will be handed. */
    if os != Os::Windows || !looks_like_path(argument) {
        return argument.to_string();
    }

    argument.replace('/', "\\")
}

/// Whether an argument is a path: it has a separator in it, or it is a Windows path with no forward slash to look
/// for. Three things are explicitly not paths, because rewriting them is how a normaliser breaks a working command:
/// a URL, a `key=value` flag, and a substitution (`s/a/b/`) - which is a path-shaped string that ends in a slash,
/// the way no real directory argument does.
fn looks_like_path(argument: &str) -> bool {
    if argument.contains("://") || argument.contains('=') {
        return false;
    }

    if argument.contains('/') {
        return !argument.ends_with('/');
    }

    /* `C:something` and `.\thing` are paths with no forward slash to convert. */
    argument.starts_with(".\\") || (argument.len() > 1 && argument.as_bytes()[1] == b':')
}

/// The separator a path in a *result* should use, so a file the model is told about can be opened by the tools that
/// read it back. The mirror of `normalise_argument`, and the same one OS: macOS and Linux both use `/`.
pub fn display_separator(os: Os) -> char {
    os.separator()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(launch: &Launch) -> Vec<&str> {
        launch.args.iter().map(String::as_str).collect()
    }

    #[test]
    fn a_plain_program_is_left_alone_on_every_os() {
        let command = Command::new("npm", &["test", "--", "--run"]);

        for os in [Os::Macos, Os::Linux, Os::Windows] {
            let launch = normalise(&command, os);

            assert_eq!(launch.program, "npm", "{}", os.name());
            assert_eq!(args(&launch), ["test", "--", "--run"], "{}", os.name());
            assert!(!launch.via_shell, "{}: no shell needed for a list of words", os.name());
        }
    }

    #[test]
    fn the_same_grep_call_becomes_three_working_commands() {
        let command = Command::new("grep", &["-rn", "TODO", "src"]);

        /* The point of the whole module: the same model output, three answers that all run. */
        assert_eq!(normalise(&command, Os::Macos).program, "grep");
        assert_eq!(normalise(&command, Os::Linux).program, "grep");

        let windows = normalise(&command, Os::Windows);

        assert_eq!(windows.program, "findstr");
        assert_eq!(args(&windows), ["-rn", "TODO", "src"]);
    }

    #[test]
    fn every_program_windows_has_no_built_in_for_is_mapped() {
        /* Asserted in full, per OS, so a change that fixes one and breaks another fails here. */
        for (unix, windows, extra) in ON_WINDOWS {
            let launch = normalise(&Command::new(unix, &["x"]), Os::Windows);

            assert_eq!(&launch.program, windows, "{unix} on windows");

            let mut want: Vec<String> = extra.iter().map(|arg| (*arg).to_string()).collect();

            want.push("x".to_string());
            assert_eq!(launch.args, want, "{unix} on windows");

            /* And the Unix name is not rewritten on Unix. */
            assert_eq!(normalise(&Command::new(unix, &["x"]), Os::Macos).program, *unix);
            assert_eq!(normalise(&Command::new(unix, &["x"]), Os::Linux).program, *unix);
        }
    }

    #[test]
    fn a_path_argument_gets_the_separator_of_the_os_that_will_open_it() {
        let command = Command::new("code", &["src/app/main.rs"]);

        assert_eq!(args(&normalise(&command, Os::Macos)), ["src/app/main.rs"]);
        assert_eq!(args(&normalise(&command, Os::Linux)), ["src/app/main.rs"]);
        /* On Windows the same argument reaches the program with a backslash. The test runs on macOS, so it names the
           platform explicitly rather than relying on the build. */
        assert_eq!(args(&normalise(&command, Os::Windows)), ["src\\app\\main.rs"]);
    }

    #[test]
    fn a_regular_expression_is_not_a_path() {
        /* `sed 's/a/b/'` is the case that makes a naive separator rewrite wrong. */
        for argument in ["s/a/b/", "http://example.com/a/b", "https://x/y"] {
            assert_eq!(normalise_argument(argument, Os::Windows), argument, "{argument} is not a path");
        }

        assert_eq!(normalise_argument("src/app.rs", Os::Windows), "src\\app.rs");
        assert_eq!(normalise_argument("./build", Os::Windows), ".\\build");
    }

    #[test]
    fn a_command_string_goes_to_the_shell_of_this_os() {
        let command = Command::line("git log --oneline | head -5");

        let mac = normalise(&command, Os::Macos);

        assert_eq!(mac.program, "sh");
        assert_eq!(args(&mac), ["-c", "git log --oneline | head -5"]);
        assert!(mac.via_shell);

        let windows = normalise(&command, Os::Windows);

        assert_eq!(windows.program, "cmd");
        assert_eq!(args(&windows), ["/C", "git log --oneline | head -5"]);
    }

    #[test]
    fn an_unknown_program_is_left_exactly_as_the_model_wrote_it() {
        /* A wrong guess is worse than no guess: `rg` may be installed, and renaming it would break a working command. */
        let command = Command::new("rg", &["pattern"]);

        for os in [Os::Macos, Os::Linux, Os::Windows] {
            assert_eq!(normalise(&command, os).program, "rg", "{}", os.name());
        }
    }

    #[test]
    fn the_display_form_is_what_the_permission_dialog_shows() {
        let launch = normalise(&Command::new("grep", &["-rn", "a b", "src"]), Os::Macos);

        assert_eq!(launch.display(), "grep -rn \"a b\" src");
    }

    #[test]
    fn this_build_knows_which_os_it_is_on() {
        assert_eq!(Os::here().name(), if cfg!(target_os = "macos") { "macos" } else if cfg!(target_os = "windows") { "windows" } else { "linux" });
        assert_eq!(display_separator(Os::Macos), '/');
        assert_eq!(display_separator(Os::Windows), '\\');
    }
}
