//! `dst` is the course runner. It owns the loop that a learner otherwise
//! runs by hand: `init` writes the state file, `start NN` copies a module's
//! acceptance tests into `project/`, `check` runs the module's `check.sh`
//! and prints one line per step, and `next` records the pass and starts the
//! next module. The state lives in `project/.dst-state`.
//!
//! The crate has no dependencies and needs nothing newer than Rust 1.70.
//! Its `rust-toolchain.toml` pins the course's toolchain only so that
//! `cargo install` and `mise run dst` install no second one.

use std::env;
use std::fs;
use std::io::{self, BufRead, BufReader, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

const LAST_MODULE: u8 = 13;
const STATE_FILE: &str = ".dst-state";
const MANIFEST_FILE: &str = "manifest";
const EXTRA_TARGET: &str = "thumbv7em-none-eabihf";
/// Width of the step name column in `dst check`.
const STEP_COLUMN: usize = 64;

const USAGE: &str = "\
dst, the course runner

usage:
  dst                      show where you are and the next command
  dst init [--force]       write the state file and check the toolchain
  dst start NN [flags]     copy module NN's tests into project/ and make it current
      --from-solution      first copy solutions/<NN-1> over project/ (files with the
                           same path are replaced, nothing is deleted)
      --force              start NN although NN-1 is not done, or do NN again
  dst check                run project/scripts/check.sh, one line per step
  dst next [--force]       record the pass, then start the next module
  dst list                 every module as done, current or pending
  dst help                 this text

For course authors:
  dst dev verify           check every manifest against the reference solutions

Run it from anywhere inside the repository, or set DST_ROOT.";

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    match run(&args) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(message) => {
            eprintln!("dst: {message}");
            ExitCode::FAILURE
        }
    }
}

/// `Ok(true)` is success, `Ok(false)` is a clean failure that was already
/// reported, such as a failing check. `Err` is a usage or I/O problem.
fn run(args: &[String]) -> Result<bool, String> {
    let verb = args.first().map(String::as_str).unwrap_or("");
    if matches!(verb, "help" | "--help" | "-h") {
        println!("{USAGE}");
        return Ok(true);
    }
    let course = Course::load()?;
    let flags: Vec<&str> = args.iter().skip(1).map(String::as_str).collect();
    match verb {
        "" | "status" => course.status(),
        "init" => course.init(has_flag(&flags, "--force")),
        "start" => {
            let number = flags
                .iter()
                .find(|f| !f.starts_with("--"))
                .ok_or("start needs a module number, for example `dst start 05`")?;
            let number = parse_module_number(number)?;
            course.start(
                number,
                has_flag(&flags, "--from-solution"),
                has_flag(&flags, "--force"),
            )
        }
        "check" => course.check(),
        "next" => course.next(has_flag(&flags, "--force")),
        "list" => course.list(),
        "dev" => match flags.first().copied() {
            Some("verify") => course.dev_verify(),
            _ => Err("dev takes one subcommand: verify".to_string()),
        },
        other => Err(format!("unknown command `{other}`\n\n{USAGE}")),
    }
}

fn has_flag(flags: &[&str], flag: &str) -> bool {
    flags.contains(&flag)
}

/// Accepts `5`, `05` and `05-sim-network`.
fn parse_module_number(text: &str) -> Result<u8, String> {
    let digits: String = text.chars().take_while(char::is_ascii_digit).collect();
    let number: u8 = digits
        .parse()
        .map_err(|_| format!("`{text}` is not a module number"))?;
    if number > LAST_MODULE {
        return Err(format!(
            "there is no module {number}; the last one is {LAST_MODULE}"
        ));
    }
    Ok(number)
}

struct Module {
    number: u8,
    /// The directory name, such as `05-sim-network`.
    slug: String,
}

impl Module {
    fn name(&self) -> &str {
        // Past the two digits and the dash.
        &self.slug[3..]
    }
}

struct Course {
    root: PathBuf,
    modules: Vec<Module>,
}

#[derive(Default)]
struct State {
    current: Option<u8>,
    done: Vec<u8>,
    /// The module whose `check` last passed.
    passed: Option<u8>,
}

impl Course {
    fn load() -> Result<Course, String> {
        let root = find_root()?;
        let mut modules = Vec::new();
        let entries =
            fs::read_dir(root.join("modules")).map_err(|e| format!("cannot read modules/: {e}"))?;
        for entry in entries {
            let entry = entry.map_err(|e| format!("cannot read modules/: {e}"))?;
            let slug = entry.file_name().to_string_lossy().into_owned();
            let is_module = slug.len() > 3
                && slug.as_bytes()[2] == b'-'
                && slug[..2].bytes().all(|b| b.is_ascii_digit());
            if is_module && entry.path().is_dir() {
                let number = slug[..2]
                    .parse()
                    .map_err(|_| format!("bad module name {slug}"))?;
                modules.push(Module { number, slug });
            }
        }
        modules.sort_by_key(|m| m.number);
        if modules.is_empty() {
            return Err("no modules found under modules/".to_string());
        }
        Ok(Course { root, modules })
    }

    fn module(&self, number: u8) -> Result<&Module, String> {
        self.modules
            .iter()
            .find(|m| m.number == number)
            .ok_or_else(|| format!("module {number:02} has no directory under modules/"))
    }

    fn project(&self) -> PathBuf {
        self.root.join("project")
    }

    fn module_dir(&self, module: &Module) -> PathBuf {
        self.root.join("modules").join(&module.slug)
    }

    fn solution_dir(&self, module: &Module) -> PathBuf {
        self.root.join("solutions").join(&module.slug)
    }

    fn lesson(&self, module: &Module) -> String {
        format!("modules/{}/README.md", module.slug)
    }

    // ---- state -----------------------------------------------------------

    fn state_path(&self) -> PathBuf {
        self.project().join(STATE_FILE)
    }

    fn load_state(&self) -> Result<State, String> {
        let path = self.state_path();
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                return Err("not initialized; run `dst init` first".to_string());
            }
            Err(e) => return Err(format!("cannot read {}: {e}", path.display())),
        };
        let mut state = State::default();
        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let value = value.trim();
            match key.trim() {
                "current" => state.current = Some(parse_module_number(value)?),
                "passed" => state.passed = Some(parse_module_number(value)?),
                "done" => {
                    for item in value.split_whitespace() {
                        state.done.push(parse_module_number(item)?);
                    }
                }
                _ => {}
            }
        }
        Ok(state)
    }

    fn save_state(&self, state: &State) -> Result<(), String> {
        let mut text = String::from("# Written by dst. `dst` with no arguments explains it.\n");
        if let Some(current) = state.current {
            text.push_str(&format!("current = {current:02}\n"));
        }
        let done: Vec<String> = state.done.iter().map(|n| format!("{n:02}")).collect();
        text.push_str(&format!("done = {}\n", done.join(" ")));
        if let Some(passed) = state.passed {
            text.push_str(&format!("passed = {passed:02}\n"));
        }
        let path = self.state_path();
        fs::write(&path, text).map_err(|e| format!("cannot write {}: {e}", path.display()))
    }

    // ---- commands --------------------------------------------------------

    fn init(&self, force: bool) -> Result<bool, String> {
        let project = self.project();
        if !project.join("Cargo.toml").is_file() {
            return Err(format!(
                "{} has no Cargo.toml; the Module 1 skeleton is missing",
                project.display()
            ));
        }
        if self.state_path().exists() && !force {
            return Err(
                "already initialized; `dst` shows where you are, and `dst init --force` \
                 starts the count over (your code is kept)"
                    .to_string(),
            );
        }
        // From project/, so that rustup reports the toolchain that the
        // workspace pins, not the default one.
        let cargo = Command::new("cargo")
            .arg("--version")
            .current_dir(&project)
            .output();
        match cargo {
            Ok(out) if out.status.success() => {
                print!("cargo:   {}", String::from_utf8_lossy(&out.stdout));
            }
            _ => return Err("cargo is not on PATH; install Rust with rustup first".to_string()),
        }
        let targets = Command::new("rustup")
            .args(["target", "list", "--installed"])
            .current_dir(&project)
            .output();
        let has_target = targets
            .map(|out| {
                String::from_utf8_lossy(&out.stdout)
                    .lines()
                    .any(|l| l.trim() == EXTRA_TARGET)
            })
            .unwrap_or(false);
        if has_target {
            println!("target:  {EXTRA_TARGET} is installed");
        } else {
            println!("target:  {EXTRA_TARGET} is not installed. Module 3 and later need it.");
            println!("         Install it with: rustup target add {EXTRA_TARGET}");
        }
        let first = self.module(0)?;
        self.save_state(&State {
            current: Some(0),
            done: Vec::new(),
            passed: None,
        })?;
        println!("state:   {}", self.state_path().display());
        println!();
        println!(
            "You are on Module 0. Read {} and then run `dst start 00`.",
            self.lesson(first)
        );
        Ok(true)
    }

    fn status(&self) -> Result<bool, String> {
        let state = self.load_state()?;
        let Some(current) = state.current else {
            return Err("the state file has no current module; run `dst init --force`".to_string());
        };
        let module = self.module(current)?;
        let done = state.done.len();
        println!(
            "Module {current:02} {} is current. {done} of {} modules are done.",
            module.name(),
            self.modules.len()
        );
        println!("lesson:  {}", self.lesson(module));
        if current == 0 {
            println!("check:   none; Module 0 has no automated check");
            println!("next:    dst next   (once NOTES.md is written in your copy of the starter)");
        } else if state.passed == Some(current) {
            println!("check:   passed");
            if current == LAST_MODULE {
                println!("next:    dst next   (the course is complete after this)");
            } else {
                println!("next:    dst next");
            }
        } else {
            println!("check:   not passed yet");
            println!("next:    dst check");
        }
        Ok(true)
    }

    fn list(&self) -> Result<bool, String> {
        let state = self.load_state()?;
        for module in &self.modules {
            let mark = if state.done.contains(&module.number) {
                "done   "
            } else if state.current == Some(module.number) {
                "current"
            } else {
                "pending"
            };
            let passed =
                if state.passed == Some(module.number) && !state.done.contains(&module.number) {
                    "  (check passed)"
                } else {
                    ""
                };
            println!("{mark}  {:02} {}{passed}", module.number, module.name());
        }
        Ok(true)
    }

    fn start(&self, number: u8, from_solution: bool, force: bool) -> Result<bool, String> {
        let mut state = self.load_state()?;
        let module = self.module(number)?;
        if state.done.contains(&number) && !force {
            let current = state.current.unwrap_or(number);
            if number == 0 {
                return Err(
                    "module 00 is done; `dst start 00 --force` makes it current again".to_string(),
                );
            }
            return Err(format!(
                "module {number:02} is done. `dst start {number:02} --force` does it again: its tests \
                 and scripts replace the newer ones in project/, and `dst start {current:02} --force` \
                 brings those back. To see what it copies, read modules/{}/{MANIFEST_FILE}.",
                module.slug
            ));
        }
        if number > 0 {
            let previous = number - 1;
            let previous_done = state.done.contains(&previous);
            if from_solution {
                if previous == 0 {
                    return Err(
                        "module 00 has no reference solution, because its task is outside the \
                         workspace. project/ already is the Module 1 starting point, so run \
                         `dst start 01`"
                            .to_string(),
                    );
                }
                let source = self.module(previous)?;
                self.copy_solution(source)?;
                if !previous_done {
                    state.done.push(previous);
                }
            } else if !previous_done && state.current != Some(number) && !force {
                return Err(format!(
                    "module {previous:02} is not done. Finish it with `dst check` and `dst next`, \
                     start from its reference solution with `dst start {number:02} --from-solution`, \
                     or copy the tests anyway with `dst start {number:02} --force`"
                ));
            }
        } else if from_solution {
            return Err("module 00 has nothing before it to start from".to_string());
        }
        self.apply_manifest(module)?;
        state.current = Some(number);
        // Starting a module again means doing it again.
        state.done.retain(|done| *done != number);
        if state.passed != Some(number) {
            state.passed = None;
        }
        self.save_state(&state)?;
        println!();
        println!("Module {number:02} {} is current.", module.name());
        println!(
            "lesson:  {}  (the task is under \"Coding task\")",
            self.lesson(module)
        );
        if number == 0 {
            println!(
                "starter: modules/{}/starter/  (copy it to a directory of your choice)",
                module.slug
            );
            println!(
                "next:    write NOTES.md in your copy, then `dst next`; there is no `dst check`"
            );
        } else {
            println!("next:    `dst check` when you want a verdict");
        }
        Ok(true)
    }

    fn check(&self) -> Result<bool, String> {
        let mut state = self.load_state()?;
        let Some(current) = state.current else {
            return Err("no current module; run `dst start NN`".to_string());
        };
        let module = self.module(current)?;
        if current == 0 {
            println!(
                "Module 00 has no automated check. Its deliverable is NOTES.md in your copy of \
                 modules/{}/starter/. Run `dst next` when it is written.",
                module.slug
            );
            return Ok(true);
        }
        let project = self.project();
        if !project.join("scripts").join("check.sh").is_file() {
            return Err(format!(
                "project/scripts/check.sh is missing; run `dst start {current:02}`"
            ));
        }
        let passed = run_check_script(&project)?;
        if passed {
            state.passed = Some(current);
            self.save_state(&state)?;
            println!();
            println!("Module {current:02} passed. Run `dst next` to move on.");
        } else {
            println!();
            println!(
                "Hints are under \"Hints\" in {}. Each requirement is numbered there.",
                self.lesson(module)
            );
        }
        Ok(passed)
    }

    fn next(&self, force: bool) -> Result<bool, String> {
        let mut state = self.load_state()?;
        let Some(current) = state.current else {
            return Err("no current module; run `dst start NN`".to_string());
        };
        // Module 0 has no check, so `next` is its only gate.
        if current > 0 && state.passed != Some(current) && !force {
            return Err(format!(
                "module {current:02} has not passed `dst check`; run it, or `dst next --force` to move on anyway"
            ));
        }
        let module = self.module(current)?;
        if !state.done.contains(&current) {
            state.done.push(current);
        }
        self.save_state(&state)?;
        println!("Module {current:02} {} is done.", module.name());
        if current > 0 {
            println!(
                "Compare your code with the reference solution in solutions/{}/ if you like.",
                module.slug
            );
        }
        if current == LAST_MODULE {
            println!();
            println!("That was the last module. The course is complete.");
            return Ok(true);
        }
        println!();
        // Forced, because the next module may be done already when the
        // learner came back to redo this one.
        self.start(current + 1, false, true)
    }

    fn dev_verify(&self) -> Result<bool, String> {
        let mut problems = 0;
        for module in &self.modules {
            if module.number == 0 {
                continue;
            }
            let module_dir = self.module_dir(module);
            let entries = read_manifest(&module_dir)?;
            let mut listed: Vec<PathBuf> = Vec::new();
            for entry in &entries {
                let (source, destination) = match entry {
                    Entry::Copy {
                        source,
                        destination,
                    } => (source, destination),
                    Entry::Delete(path) => {
                        let in_solution = self.solution_dir(module).join(path);
                        if in_solution.exists() {
                            problems += 1;
                            println!(
                                "{}: the manifest deletes {}, but the solution has it",
                                module.slug,
                                path.display()
                            );
                        }
                        continue;
                    }
                };
                listed.push(source.clone());
                let in_module = module_dir.join(source);
                let in_solution = self.solution_dir(module).join(destination);
                match (fs::read(&in_module), fs::read(&in_solution)) {
                    (Ok(a), Ok(b)) if a == b => {}
                    (Ok(_), Ok(_)) => {
                        problems += 1;
                        println!(
                            "{}: {} differs from {}",
                            module.slug,
                            source.display(),
                            in_solution.display()
                        );
                    }
                    (Err(e), _) => {
                        problems += 1;
                        println!("{}: cannot read {}: {e}", module.slug, in_module.display());
                    }
                    (_, Err(e)) => {
                        problems += 1;
                        println!(
                            "{}: cannot read {}: {e}",
                            module.slug,
                            in_solution.display()
                        );
                    }
                }
            }
            let mut present = Vec::new();
            walk(&module_dir, &module_dir, &mut present)?;
            for file in present {
                let name = file.to_string_lossy();
                if name == "README.md" || name == MANIFEST_FILE || name.starts_with("starter/") {
                    continue;
                }
                if !listed.contains(&file) {
                    problems += 1;
                    println!("{}: {} is not in the manifest", module.slug, file.display());
                }
            }
            println!("{}: {} files checked", module.slug, entries.len());
        }
        if problems == 0 {
            println!("every manifest matches its reference solution");
            Ok(true)
        } else {
            println!("{problems} problems");
            Ok(false)
        }
    }

    // ---- helpers ---------------------------------------------------------

    fn apply_manifest(&self, module: &Module) -> Result<(), String> {
        let module_dir = self.module_dir(module);
        if !module_dir.join(MANIFEST_FILE).is_file() {
            if module.number == 0 {
                return Ok(());
            }
            return Err(format!(
                "modules/{}/{MANIFEST_FILE} is missing",
                module.slug
            ));
        }
        let project = self.project();
        for entry in read_manifest(&module_dir)? {
            let (source, destination) = match entry {
                Entry::Copy {
                    source,
                    destination,
                } => (source, destination),
                Entry::Delete(path) => {
                    let target = project.join(&path);
                    if target.is_file() {
                        fs::remove_file(&target)
                            .map_err(|e| format!("cannot delete {}: {e}", target.display()))?;
                        println!(
                            "deleted   project/{}  (an older test this module retires)",
                            path.display()
                        );
                    }
                    continue;
                }
            };
            let from = module_dir.join(&source);
            let to = project.join(&destination);
            let content =
                fs::read(&from).map_err(|e| format!("cannot read {}: {e}", from.display()))?;
            let verdict = match fs::read(&to) {
                Ok(existing) if existing == content => "unchanged",
                Ok(_) => "replaced",
                Err(_) => "new",
            };
            if verdict != "unchanged" {
                if let Some(parent) = to.parent() {
                    fs::create_dir_all(parent)
                        .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
                }
                fs::write(&to, &content)
                    .map_err(|e| format!("cannot write {}: {e}", to.display()))?;
                copy_permissions(&from, &to)?;
            }
            println!("{verdict:<9} project/{}", destination.display());
        }
        Ok(())
    }

    fn copy_solution(&self, module: &Module) -> Result<(), String> {
        let source = self.solution_dir(module);
        if !source.join("Cargo.toml").is_file() {
            return Err(format!("{} has no Cargo.toml", source.display()));
        }
        let mut files = Vec::new();
        walk(&source, &source, &mut files)?;
        let project = self.project();
        let mut copied = 0;
        for relative in files {
            let from = source.join(&relative);
            let to = project.join(&relative);
            if let Some(parent) = to.parent() {
                fs::create_dir_all(parent)
                    .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
            }
            fs::copy(&from, &to)
                .map_err(|e| format!("cannot copy {} to {}: {e}", from.display(), to.display()))?;
            copied += 1;
        }
        println!(
            "copied {copied} files from solutions/{}/ into project/ (files with the same path were \
             replaced, nothing was deleted)",
            module.slug
        );
        Ok(())
    }
}

/// One line of a module's manifest.
enum Entry {
    /// `source -> destination`: copy a file from the module directory to
    /// `project/`.
    Copy {
        source: PathBuf,
        destination: PathBuf,
    },
    /// `delete path`: remove a file under `project/` that an earlier module
    /// placed and this one retires.
    Delete(PathBuf),
}

/// Reads `manifest` in a module directory. Each line is
/// `source -> destination`, with the source relative to the module directory
/// and the destination relative to `project/`, or `delete path` with the
/// path relative to `project/`. Blank lines and `#` comments are skipped.
fn read_manifest(module_dir: &Path) -> Result<Vec<Entry>, String> {
    let path = module_dir.join(MANIFEST_FILE);
    let text =
        fs::read_to_string(&path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let mut entries = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(target) = line.strip_prefix("delete ") {
            entries.push(Entry::Delete(PathBuf::from(target.trim())));
            continue;
        }
        let (source, destination) = line.split_once(" -> ").ok_or_else(|| {
            format!(
                "{}:{}: expected `source -> destination` or `delete path`",
                path.display(),
                index + 1
            )
        })?;
        entries.push(Entry::Copy {
            source: PathBuf::from(source.trim()),
            destination: PathBuf::from(destination.trim()),
        });
    }
    Ok(entries)
}

/// Every file under `dir`, relative to `base`, skipping build and version
/// control directories and the state file.
fn walk(base: &Path, dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    let entries = fs::read_dir(dir).map_err(|e| format!("cannot read {}: {e}", dir.display()))?;
    let mut paths: Vec<PathBuf> = entries
        .map(|entry| entry.map(|e| e.path()))
        .collect::<Result<_, _>>()
        .map_err(|e| format!("cannot read {}: {e}", dir.display()))?;
    paths.sort();
    for path in paths {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        if path.is_dir() {
            if name == "target" || name == ".git" || name == ".jj" {
                continue;
            }
            walk(base, &path, out)?;
        } else if name != STATE_FILE {
            let relative = path
                .strip_prefix(base)
                .map_err(|e| e.to_string())?
                .to_path_buf();
            out.push(relative);
        }
    }
    Ok(())
}

/// Scripts must stay executable when they are copied.
fn copy_permissions(from: &Path, to: &Path) -> Result<(), String> {
    let permissions = fs::metadata(from)
        .map_err(|e| format!("cannot stat {}: {e}", from.display()))?
        .permissions();
    fs::set_permissions(to, permissions).map_err(|e| format!("cannot chmod {}: {e}", to.display()))
}

/// Counts the `echo "== step"` lines of a check script, minus the closing
/// "all checks passed", so that `dst check` can say how many steps there are.
fn count_steps(script: &Path) -> usize {
    fs::read_to_string(script)
        .map(|text| {
            text.lines()
                .filter(|line| line.trim_start().starts_with("echo \"== "))
                .filter(|line| !line.contains("all checks passed"))
                .count()
        })
        .unwrap_or(0)
}

/// A clock that ticks on the step line while a step runs, so that a long
/// step looks busy and not hung. It only draws when stdout is a terminal.
struct Ticker {
    stop: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl Ticker {
    fn start(name: &str) -> Ticker {
        let stop = Arc::new(AtomicBool::new(false));
        if !io::stdout().is_terminal() {
            return Ticker { stop, handle: None };
        }
        let flag = Arc::clone(&stop);
        let line = format!("{name:<STEP_COLUMN$} ");
        let began = Instant::now();
        let handle = thread::spawn(move || {
            while !flag.load(Ordering::Relaxed) {
                thread::sleep(Duration::from_secs(1));
                if flag.load(Ordering::Relaxed) {
                    break;
                }
                let seconds = began.elapsed().as_secs();
                print!("\r{line}{seconds}s");
                let _ = io::stdout().flush();
            }
        });
        Ticker {
            stop,
            handle: Some(handle),
        }
    }

    /// Stops the clock and leaves the cursor at the end of the step name.
    fn stop(mut self, name: &str) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
            // Overwrite the seconds with spaces, then return to the column.
            print!("\r{name:<STEP_COLUMN$}          \r{name:<STEP_COLUMN$} ");
            let _ = io::stdout().flush();
        }
    }
}

/// Runs `scripts/check.sh` in `project` and prints one line per `== step`
/// header, with the time the step took. The output of a step is shown only
/// when that step fails.
fn run_check_script(project: &Path) -> Result<bool, String> {
    let total = count_steps(&project.join("scripts").join("check.sh"));
    if total > 0 {
        let plural = if total == 1 { "" } else { "s" };
        println!("{total} step{plural}. A step prints nothing until it ends; the slow ones take minutes.");
    }
    let mut child = Command::new("bash")
        .arg("-c")
        .arg("exec scripts/check.sh 2>&1")
        .current_dir(project)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|e| format!("cannot run scripts/check.sh: {e}"))?;
    let stdout = child.stdout.take().ok_or("no stdout from check.sh")?;
    let mut out = io::stdout();
    // The name of the open step, its clock, and when it began.
    let mut open: Option<(String, Ticker, Instant)> = None;
    let mut steps_run = 0;
    let mut buffer: Vec<String> = Vec::new();
    for line in BufReader::new(stdout).lines() {
        let line = line.map_err(|e| format!("cannot read check.sh output: {e}"))?;
        if let Some(name) = line.strip_prefix("== ") {
            if let Some((name, ticker, began)) = open.take() {
                ticker.stop(&name);
                println!("ok    {}", elapsed(began));
            }
            buffer.clear();
            if name == "all checks passed" {
                continue;
            }
            steps_run += 1;
            print!("{name:<STEP_COLUMN$} ");
            out.flush().map_err(|e| e.to_string())?;
            open = Some((name.to_string(), Ticker::start(name), Instant::now()));
        } else {
            buffer.push(line);
        }
    }
    let status = child
        .wait()
        .map_err(|e| format!("cannot wait for check.sh: {e}"))?;
    let last = open.take().map(|(name, ticker, began)| {
        ticker.stop(&name);
        began
    });
    if status.success() {
        if let Some(began) = last {
            println!("ok    {}", elapsed(began));
        }
        println!("all checks passed");
        return Ok(true);
    }
    if let Some(began) = last {
        println!("FAIL  {}", elapsed(began));
    } else {
        println!("FAIL before the first step");
    }
    for line in &buffer {
        println!("    {line}");
    }
    if total > steps_run {
        let left = total - steps_run;
        let plural = if left == 1 { "" } else { "s" };
        println!();
        println!("Step {steps_run} of {total} failed; {left} later step{plural} did not run.");
    }
    Ok(false)
}

/// Seconds since `began`, with one decimal under a minute.
fn elapsed(began: Instant) -> String {
    let seconds = began.elapsed().as_secs_f64();
    if seconds < 60.0 {
        format!("{seconds:.1}s")
    } else {
        format!("{}m{:02}s", seconds as u64 / 60, seconds as u64 % 60)
    }
}

/// The repository root is the first directory, from `DST_ROOT` or the
/// working directory upward, that holds both `modules/` and `project/`.
fn find_root() -> Result<PathBuf, String> {
    if let Ok(root) = env::var("DST_ROOT") {
        let root = PathBuf::from(root);
        if root.join("modules").is_dir() && root.join("project").is_dir() {
            return Ok(root);
        }
        return Err(format!(
            "DST_ROOT={} has no modules/ and project/",
            root.display()
        ));
    }
    let start =
        env::current_dir().map_err(|e| format!("cannot read the working directory: {e}"))?;
    let mut dir: Option<&Path> = Some(&start);
    while let Some(candidate) = dir {
        if candidate.join("modules").is_dir() && candidate.join("project").is_dir() {
            return Ok(candidate.to_path_buf());
        }
        dir = candidate.parent();
    }
    Err("not inside the course repository (no directory with modules/ and project/ above here); set DST_ROOT".to_string())
}
