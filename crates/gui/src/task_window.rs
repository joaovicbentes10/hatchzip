//! The small window behind "Extract here", "Extract to a folder", "Extract
//! files" and "Test". The right-click was the confirmation, so it starts at
//! once, unless the space check says no. As with WinRAR, the window only
//! appears once the task has run for a moment, asks something, or has an
//! outcome to show, so a small archive extracts without one. Several
//! archives run one after another, each checked just before it starts; one
//! that doesn't finish stops the rest until it is resumed.

use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use anyhow::Result;
use hatchzip_core::session;
use slint::{CloseRequestResponse, ComponentHandle, Timer, TimerMode};

use crate::args::{TaskArgs, TaskKind};
use crate::runner::Runner;
use crate::settings::Settings;
use crate::task::Task;
use crate::work::Request;
use crate::{Phase, TaskWindow};

/// Where each kind of task puts the files. `None` for a test, and for files
/// whose folder is not chosen yet.
pub fn output_for(kind: &TaskKind, primary: &Path) -> Option<PathBuf> {
    match kind {
        TaskKind::ExtractHere => Some(primary.parent().unwrap_or(Path::new(".")).to_path_buf()),
        TaskKind::ExtractTo => Some(session::default_output(primary)),
        TaskKind::ExtractFiles(folder) => folder.clone(),
        TaskKind::Test => None,
    }
}

pub fn run(mut args: TaskArgs) -> Result<()> {
    if args.kind == TaskKind::ExtractFiles(None) {
        let mut dialog = rfd::FileDialog::new().set_title("Select destination folder");
        if let Some(dir) = args
            .archives
            .first()
            .and_then(|a| std::path::absolute(a).ok())
            .and_then(|a| a.parent().map(Path::to_path_buf))
        {
            dialog = dialog.set_directory(dir);
        }
        // Closing the picker is how to change one's mind; nothing was read.
        let Some(folder) = dialog.pick_folder() else {
            return Ok(());
        };
        args.kind = TaskKind::ExtractFiles(Some(folder));
    }
    let settings = Settings::saved();
    let close_when_done = settings.close_when_done;
    let (ui, start, runner) = prepare(args, settings)?;
    // Task windows draw in software (see main.rs), which has no first frame
    // to wait for, so this starts the task while the window is still hidden.
    crate::backend::when_drawn(ui.window(), start);
    let _appear = appear_when_needed(&ui, runner, close_when_done);
    slint::run_event_loop_until_quit()?;
    Ok(())
}

/// How long a task runs before its window appears.
const SHOW_AFTER: Duration = Duration::from_secs(1);

/// Shows the hidden window once the task has run for [`SHOW_AFTER`], asks
/// for a password or about existing files, or stops with something to see:
/// a test's result, a failure, a pause, too little space, or a finished
/// extraction when the window stays open. A finished extraction that closes
/// the window never shows it.
fn appear_when_needed(ui: &TaskWindow, runner: Rc<Runner>, close_when_done: bool) -> Timer {
    let started = Instant::now();
    let shown = Cell::new(false);
    let weak = ui.as_weak();
    let timer = Timer::default();
    timer.start(TimerMode::Repeated, Duration::from_millis(50), move || {
        let Some(ui) = weak.upgrade() else { return };
        if shown.get() {
            return;
        }
        let busy = matches!(
            ui.get_task().phase,
            Phase::Idle | Phase::Checking | Phase::Verifying | Phase::Working | Phase::Pausing
        );
        let closing = close_when_done && runner.task.borrow().as_ref().is_some_and(Task::extracted);
        let asking = ui.get_password_shown() || ui.get_replace_shown();
        if asking || (busy && started.elapsed() >= SHOW_AFTER) || (!busy && !closing) {
            shown.set(true);
            let _ = ui.show();
        }
    });
    timer
}

/// The task and request for one archive. The main window's reserved space
/// and verification settings apply. The archive is kept when the menu entry
/// says so or when the main window's setting does, so someone who never
/// wants archives removed gets that from every entry.
fn job_for(
    kind: &TaskKind,
    archive: &Path,
    keep_archive: bool,
    settings: &Settings,
) -> (Task, Request) {
    // Only resolves the path; a missing file is reported by the run itself.
    let primary = session::locate(archive).map_or_else(|_| archive.to_path_buf(), |(p, _)| p);
    let name = primary
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let archive = archive.to_path_buf();
    match output_for(kind, &primary) {
        Some(out) => (
            Task::extract(&name, Some(&out), keep_archive),
            Request::Extract {
                archive,
                output: Some(out),
                keep_free: settings.keep_free,
                verify_first: settings.verify_first,
                keep_archive,
                password: None,
                here: *kind == TaskKind::ExtractHere,
            },
        ),
        None => (
            Task::test(&name),
            Request::Test {
                archive,
                password: None,
            },
        ),
    }
}

/// The window, shown, and the function that starts the task. Nothing
/// touches an archive until that is called.
pub fn build(args: TaskArgs, settings: Settings) -> Result<(TaskWindow, impl FnOnce() + 'static)> {
    let (ui, start, _) = prepare(args, settings)?;
    ui.show()?;
    Ok((ui, start))
}

/// [`build`] without showing the window, and the runner, to watch the task.
fn prepare(
    args: TaskArgs,
    settings: Settings,
) -> Result<(TaskWindow, impl FnOnce() + 'static, Rc<Runner>)> {
    if args.kind == TaskKind::ExtractFiles(None) {
        anyhow::bail!("no destination folder was chosen");
    }
    let ui = TaskWindow::new()?;
    let jobs: Vec<(Task, Request)> = args
        .archives
        .iter()
        .map(|a| {
            let keep = args.keep_archive || settings.keep_archive;
            job_for(&args.kind, a, keep, &settings)
        })
        .collect();
    let Some((task, _)) = jobs.first() else {
        anyhow::bail!("no archive given");
    };

    let weak = ui.as_weak();
    let runner = Runner::install(move |task: &Task| {
        if let Some(ui) = weak.upgrade() {
            ui.set_task(task.view.clone());
            ui.set_plan(task.plan.clone());
        }
    });
    ui.set_task(task.view.clone());
    if settings.close_when_done {
        let weak = ui.as_weak();
        runner.when_extracted(move || {
            if let Some(ui) = weak.upgrade() {
                let _ = ui.hide();
            }
            // The loop may have started with no window shown, so hiding the
            // last one doesn't end it.
            let _ = slint::quit_event_loop();
        });
    }

    let r = runner.clone();
    ui.on_pause(move || r.pause());
    let r = runner.clone();
    ui.on_resume(move || r.retry());
    let r = runner.clone();
    ui.on_open_folder(move || {
        let task = r.task.borrow();
        if let Some(out) = task.as_ref().and_then(|t| t.output.clone()) {
            let _ = open::that_detached(out);
        }
    });
    let r = runner.clone();
    ui.on_give_back(move || r.give_back());
    crate::password::host_in!(ui);
    crate::replace::host_in!(ui);
    let r = runner.clone();
    ui.on_dismiss(move || {
        if r.may_close() {
            let _ = slint::quit_event_loop();
        }
    });
    let r = runner.clone();
    ui.window().on_close_requested(move || {
        if r.may_close() {
            let _ = slint::quit_event_loop();
            CloseRequestResponse::HideWindow
        } else {
            CloseRequestResponse::KeepWindowShown
        }
    });

    let r = runner.clone();
    Ok((ui, move || r.start_queue(jobs), runner))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_task_writes_where_its_menu_entry_says() {
        let primary = Path::new("/home/me/Downloads/game.7z.001");
        assert_eq!(
            output_for(&TaskKind::ExtractHere, primary),
            Some(PathBuf::from("/home/me/Downloads"))
        );
        assert_eq!(
            output_for(&TaskKind::ExtractTo, primary),
            Some(PathBuf::from("/home/me/Downloads/game"))
        );
        let chosen = PathBuf::from("/media/usb");
        assert_eq!(
            output_for(&TaskKind::ExtractFiles(Some(chosen.clone())), primary),
            Some(chosen)
        );
        assert_eq!(output_for(&TaskKind::Test, primary), None);
    }
}
