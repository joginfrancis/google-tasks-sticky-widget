// `auth` and `google` are `pub` so the diagnostic binaries in examples/ can
// reuse the real code rather than reimplementing it and proving something
// different from what the app actually does.
pub mod auth;
pub mod google;

mod autostart;
mod hotkey;
mod notes;
mod commands;
mod settings;
mod store;
mod sync;
mod tasks_commands;
mod tray;
mod window;

use tauri::{Manager, WindowEvent};
use tauri_plugin_window_state::{Builder as WindowStateBuilder, StateFlags};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // Must be registered first, before anything else can take a lock on the
        // cache or a tray icon.
        //
        // Without this, every click of the desktop shortcut started another
        // copy: two SQLite connections, two sync loops, two tray icons, and two
        // stacks of windows on top of each other — which reads as the app being
        // frozen, because you clean up one instance while looking at another.
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            log::info!("second launch requested; focusing the existing window");
            if let Some(win) = window::main_window(app) {
                window::show_and_focus(&win);
            }
        }))
        .plugin(
            tauri_plugin_log::Builder::new()
                .level(if cfg!(debug_assertions) {
                    log::LevelFilter::Debug
                } else {
                    // ARCHITECTURE §7.3 — DEBUG off in release builds.
                    log::LevelFilter::Info
                })
                // Our own DEBUG output is reviewed; a dependency's is not, and
                // these three sit closest to credentials and request URLs.
                // Cap them at Info so nothing unvetted lands in a log file.
                .level_for("keyring", log::LevelFilter::Info)
                .level_for("keyring_core", log::LevelFilter::Info)
                .level_for("reqwest", log::LevelFilter::Info)
                .build(),
        )
        // Only position and size. Restoring visibility or decorations here would
        // fight the tray and the frameless config.
        .plugin(
            WindowStateBuilder::default()
                .with_state_flags(StateFlags::POSITION | StateFlags::SIZE)
                .build(),
        )
        .plugin(tauri_plugin_opener::init())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, _shortcut, event| {
                    // Key-down only; without this the toggle fires twice per
                    // press and the window appears to flicker.
                    if event.state == tauri_plugin_global_shortcut::ShortcutState::Pressed
                    {
                        hotkey::on_pressed(app);
                    }
                })
                .build(),
        )
        .invoke_handler(tauri::generate_handler![
            commands::set_window_layer,
            commands::get_window_layer,
            commands::set_show_in_taskbar,
            commands::hide_to_tray,
            commands::quit_app,
            commands::ui_log,
            commands::set_list_color,
            commands::get_list_colors,
            commands::get_selected_task_list,
            commands::set_selected_task_list,
            commands::get_window_settings,
            commands::set_start_hidden,
            commands::set_start_with_windows,
            commands::set_global_hotkey,
            commands::get_account_status,
            commands::begin_google_auth,
            commands::disconnect_google,
            tasks_commands::list_tasks,
            tasks_commands::list_task_lists,
            tasks_commands::refresh_task_lists,
            tasks_commands::sync_now,
            tasks_commands::create_task,
            tasks_commands::set_task_completed,
            tasks_commands::update_task,
            tasks_commands::open_task_in_google,
            tasks_commands::delete_task,
            tasks_commands::move_task,
            tasks_commands::create_task_list,
            tasks_commands::rename_task_list,
            tasks_commands::delete_task_list,
            notes::open_note_window,
            notes::note_list_id,
            notes::close_note_window,
            notes::open_note_labels,
            notes::register_note_list,
            notes::note_window_at,
            notes::note_window_frame,
            sync::note_activity,
            sync::request_sync,
            sync::sync_interval_secs,
        ])
        .setup(|app| {
            let handle = app.handle().clone();
            // Marks the start of a run, so a log file covering several launches
            // can be read without guessing where one ends.
            log::info!(
                "=== Sticky Widget starting (v{}) ===",
                env!("CARGO_PKG_VERSION")
            );
            app.manage(auth::AuthState::default());
            app.manage(notes::NoteRegistry::default());

            // A cache that cannot be opened must not stop the widget from
            // running — it degrades to network-only, which is survivable.
            let cache_path = handle
                .path()
                .app_data_dir()
                .map(|dir| dir.join("cache.sqlite"))
                .unwrap_or_else(|_| std::path::PathBuf::from("cache.sqlite"));

            match store::Store::open(cache_path) {
                Ok(store) => {
                    app.manage(store);
                }
                Err(err) => {
                    log::error!("local cache unavailable: {err}");
                    return Err(err.into());
                }
            }

            let loaded = settings::load(&handle);
            let start_hidden = loaded.start_hidden;
            let layer = loaded.layer_for(notes::MAIN_LABEL);
            let show_in_taskbar = loaded.show_in_taskbar;
            let selected_list = loaded.selected_task_list_id.clone();
            let hotkey_accelerator = loaded.effective_hotkey();
            app.manage(settings::AppState::new(loaded));

            // A moved or reinstalled binary leaves a stale Run entry that fails
            // silently at sign-in, so the user thinks the setting works.
            autostart::refresh_path_if_registered();

            // Registered after state is managed so a failure can be recorded
            // where Settings can read it.
            app.manage(hotkey::HotkeyStatus::default());
            let _ = hotkey::apply(&handle, hotkey_accelerator.as_deref());

            let manager = sync::SyncManager::new();
            manager.set_list(selected_list);
            // Starting hidden means the widget is not on screen, so the slow
            // cadence is correct from the first tick rather than after one.
            manager.set_visible(!start_hidden);
            manager.start(handle.clone());
            app.manage(manager);

            tray::build(app)?;

            if let Some(win) = window::main_window(&handle) {
                // The window-state plugin has already restored geometry by now,
                // so this is the moment to sanity-check it.
                window::ensure_on_screen(&win);

                // The main note has its own pin state like any other.
                match layer {
                    settings::WindowLayer::Top => {
                        let _ = win.set_always_on_top(true);
                    }
                    settings::WindowLayer::Normal => {
                        let _ = win.set_always_on_top(false);
                    }
                    settings::WindowLayer::Bottom => {
                        let _ = win.set_always_on_top(false);
                        let _ = win.set_always_on_bottom(true);
                    }
                }

                let _ = win.set_skip_taskbar(!show_in_taskbar);

                // Either the stored preference or the flag the Run entry passes.
                if start_hidden || autostart::launched_hidden() {
                    let _ = win.hide();
                }
            }

            // Reopen whatever was on screen last time. Safe to build windows
            // synchronously here: setup runs before the event loop starts, so
            // the deadlock that forces `open_note_window` to be async does not
            // apply. Lists are validated against the cache so a note for a
            // deleted list is dropped rather than opened empty.
            let known_lists: Vec<String> = handle
                .state::<store::Store>()
                .task_lists()
                .map(|lists| lists.into_iter().map(|l| l.id).collect())
                .unwrap_or_default();
            notes::restore_session(&handle, &known_lists);

            Ok(())
        })
        .on_window_event(|win, event| match event {
            WindowEvent::CloseRequested { api, .. } => {
                if win.label() == notes::MAIN_LABEL {
                    // The main note hides rather than closing. With skipTaskbar
                    // on, exiting here would strand the user with a
                    // running-but-invisible app.
                    api.prevent_close();
                    let _ = win.hide();
                } else {
                    // Extra notes really close — they can be reopened from any
                    // window's list menu, so nothing is lost.
                    win.state::<notes::NoteRegistry>().remove(win.label());
                    win.state::<sync::SyncManager>()
                        .refresh_lists(win.app_handle());
                }

                // Cadence follows the app, not one window: only slow down once
                // nothing is on screen.
                let any_visible = win
                    .app_handle()
                    .webview_windows()
                    .values()
                    .any(|w| w.label() != win.label() && w.is_visible().unwrap_or(false));
                if !any_visible {
                    win.state::<sync::SyncManager>().set_visible(false);
                }
            }
            // Focus is the strongest signal that freshness matters right now,
            // so it both speeds up the cadence and triggers an immediate sync.
            //
            // Losing focus deliberately does nothing: treating it as activity
            // would refresh the timer every time the user clicked another app,
            // pinning the widget at the fast cadence forever.
            WindowEvent::Focused(true) => {
                win.state::<sync::SyncManager>().set_visible(true);
                // Stands in for z-order when a drag has to choose between two
                // overlapping notes; see notes::note_window_at.
                win.state::<notes::NoteRegistry>().note_focused(win.label());
            }
            _ => {}
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
