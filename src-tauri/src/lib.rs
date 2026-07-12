use tauri::{
    image::Image,
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    Listener, Manager,
};

/// Mostra e mette a fuoco la finestra principale del planner.
fn show_main(app: &tauri::AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
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

            // Menù del tray
            let open_i = MenuItem::with_id(app, "open", "Apri PIM Tomato", true, None::<&str>)?;
            let quit_i = MenuItem::with_id(app, "quit", "Esci", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&open_i, &quit_i])?;

            // Icona 🍅 MONOCROMATICA (template) come le altre della barra dei menu
            let tray_icon = Image::from_bytes(include_bytes!("../icons/tray.png"))?;
            let _tray = TrayIconBuilder::with_id("tray")
                .icon(tray_icon)
                .icon_as_template(true)
                .tooltip("PIM Tomato")
                .menu(&menu)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "open" => show_main(app),
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
