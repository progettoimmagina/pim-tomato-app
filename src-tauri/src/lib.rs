use tauri::{
    image::Image,
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    Emitter, Listener, Manager, WebviewUrl, WebviewWindowBuilder,
};

/// Mostra e mette a fuoco la finestra principale del planner.
fn show_main(app: &tauri::AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

/// Mostra la finestrella-timer staccata (in alto a destra).
fn show_timer(app: &tauri::AppHandle) {
    if let Some(w) = app.get_webview_window("timer") {
        let _ = w.show();
        let _ = w.set_focus();
    }
}

/// Applica gli aggiornamenti richiesti dal planner (evento "pt-app"):
/// { "title": "01:59", "visible": true, "badge": 3 }. Ogni campo è opzionale
/// e viene applicato solo se presente.
fn apply_pt(app: &tauri::AppHandle, payload: &str) {
    let v: serde_json::Value = match serde_json::from_str(payload) {
        Ok(v) => v,
        Err(_) => return,
    };
    if let Some(tray) = app.tray_by_id("tray") {
        if let Some(t) = v.get("title") {
            let title = t.as_str().unwrap_or("");
            let _ = tray.set_title(if title.is_empty() { None } else { Some(title.to_string()) });
        }
        if let Some(vis) = v.get("visible").and_then(|x| x.as_bool()) {
            let _ = tray.set_visible(vis);
        }
    }
    if let Some(bv) = v.get("badge") {
        if let Some(w) = app.get_webview_window("main") {
            let n = bv.as_i64();
            let _ = w.set_badge_count(n.filter(|x| *x > 0));
        }
    }
}

/// Apre un link ESTERNO col comportamento "app ClickUp se installata, altrimenti
/// browser". Per gli URL app.clickup.com prova prima il deep-link clickup://
/// (apre l'app desktop di ClickUp); se nessun gestore è registrato, ripiega
/// sull'URL https nel browser di sistema. Gli altri link vanno al browser.
fn open_external(payload: &str) {
    let v: serde_json::Value = match serde_json::from_str(payload) {
        Ok(v) => v,
        Err(_) => return,
    };
    let url = match v.get("url").and_then(|x| x.as_str()) {
        Some(u) if !u.is_empty() => u.to_string(),
        _ => return,
    };
    if let Some(rest) = url.strip_prefix("https://app.clickup.com/") {
        let deep = format!("clickup://{}", rest);
        let opened = std::process::Command::new("open")
            .arg(&deep)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if opened {
            return;
        }
        let _ = std::process::Command::new("open").arg(&url).status(); // fallback browser
    } else {
        let _ = std::process::Command::new("open").arg(&url).status();
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            // Auto-update all'avvio: controlla, scarica, installa e riavvia (silenzioso).
            let up_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                use tauri_plugin_updater::UpdaterExt;
                if let Ok(updater) = up_handle.updater() {
                    if let Ok(Some(update)) = updater.check().await {
                        if update.download_and_install(|_, _| {}, || {}).await.is_ok() {
                            up_handle.restart();
                        }
                    }
                }
            });

            // Finestrella-timer staccata (in alto a destra, senza bordi, sempre in
            // primo piano). Vive finché l'app è viva (anche col planner nascosto).
            let timer_win = WebviewWindowBuilder::new(app, "timer", WebviewUrl::App("timer.html".into()))
                .title("PIM Tomato Timer")
                .inner_size(320.0, 182.0)
                .resizable(false)
                .decorations(false)
                .transparent(true)
                .always_on_top(true)
                .skip_taskbar(true)
                .visible(false)
                .build()?;
            if let Ok(Some(mon)) = timer_win.primary_monitor() {
                let sz = mon.size();
                let sf = mon.scale_factor();
                let lw = sz.width as f64 / sf;
                let _ = timer_win.set_position(tauri::LogicalPosition::new(lw - 320.0 - 16.0, 40.0));
            }

            // Menù del tray
            let open_i = MenuItem::with_id(app, "open", "Apri PIM Tomato", true, None::<&str>)?;
            let timer_i = MenuItem::with_id(app, "timer", "Mostra timer", true, None::<&str>)?;
            let quit_i = MenuItem::with_id(app, "quit", "Esci", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&open_i, &timer_i, &quit_i])?;

            // Icona 🍅 MONOCROMATICA (template) come le altre della barra dei menu
            let tray_icon = Image::from_bytes(include_bytes!("../icons/tray.png"))?;
            let _tray = TrayIconBuilder::with_id("tray")
                .icon(tray_icon)
                .icon_as_template(true)
                .tooltip("PIM Tomato")
                .menu(&menu)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "open" => show_main(app),
                    "timer" => show_timer(app),
                    "quit" => app.exit(0),
                    _ => {}
                })
                .build(app)?;

            // Aggiornamenti dal planner: countdown nel tray / icona / badge Dock
            let handle = app.handle().clone();
            app.listen("pt-app", move |event| {
                apply_pt(&handle, event.payload());
            });

            // Apertura link esterni (es. "apri in ClickUp") fuori dalla finestra:
            // app ClickUp se installata, altrimenti browser.
            app.listen("pt-open", move |event| {
                open_external(event.payload());
            });

            // PONTE planner → finestrella-timer: lo stato del blocco corrente
            // (task, percorso, scadenza) viaggia alla finestrella, che conta i
            // secondi da sola. Quando c'è un blocco attivo, la finestrella appare.
            let h_timer = app.handle().clone();
            app.listen("pt-timer", move |event| {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(event.payload()) {
                    if v.get("active").and_then(|x| x.as_bool()).unwrap_or(false) {
                        show_timer(&h_timer);
                    }
                    let _ = h_timer.emit_to("timer", "pt-timer", v);
                }
            });
            // PONTE finestrella → planner: i tasti (finito / pausa / ...) tornano
            // al planner che agisce sul focus. "hide" nasconde la finestrella.
            let h_act = app.handle().clone();
            app.listen("pt-timer-action", move |event| {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(event.payload()) {
                    if v.get("a").and_then(|x| x.as_str()) == Some("hide") {
                        if let Some(w) = h_act.get_webview_window("timer") { let _ = w.hide(); }
                    }
                    let _ = h_act.emit_to("main", "pt-timer-action", v);
                }
            });

            // La X rossa NON chiude l'app: nasconde solo la finestra, così tray,
            // timer e badge restano vivi. Per uscire davvero: Cmd+Q o tray "Esci".
            if let Some(win) = app.get_webview_window("main") {
                let w = win.clone();
                win.on_window_event(move |event| {
                    if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                        api.prevent_close();
                        let _ = w.hide();
                    }
                });
            }

            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("errore nell'avvio di PIM Tomato")
        .run(|app, event| {
            // Clic sull'icona nel Dock quando la finestra è nascosta → la riapre.
            if let tauri::RunEvent::Reopen { .. } = event {
                show_main(app);
            }
        });
}
