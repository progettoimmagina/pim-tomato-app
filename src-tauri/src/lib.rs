use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{
    image::Image,
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Emitter, Listener, Manager, WebviewUrl, WebviewWindowBuilder,
};

/// Timer "fissato" (pin): resta in alto a destra sopra tutto. Se falso, il box
/// vive sotto la barra dei menu e si nasconde quando perde il focus.
static PINNED: AtomicBool = AtomicBool::new(false);
/// C'era già un blocco attivo al tick precedente? Serve a mostrare la
/// finestrella SOLO quando la giornata parte (fronte), non a ogni secondo
/// (altrimenti riappare subito dopo che l'utente l'ha chiusa).
static WAS_ACTIVE: AtomicBool = AtomicBool::new(false);
/// Ignora il blur per un attimo dopo aver mostrato il box, altrimenti la
/// finestrella non fissata si richiude all'istante appena appare.
static SUPPRESS_BLUR: AtomicBool = AtomicBool::new(false);
/// C'è una NOTIFICA (promemoria) a schermo nel box: in tal caso il box non va
/// nascosto dai normali segnali del timer finché l'utente non fa "Ho capito".
static NOTIF_ACTIVE: AtomicBool = AtomicBool::new(false);

/// Mostra e mette a fuoco la finestra principale del planner.
/// Aprendo il planner, il box-timer in alto a destra sparisce (si usa la barra
/// in basso). L'app si apre SOLO da qui (Dock o freccia del box), mai dai tasti.
fn show_main(app: &tauri::AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
    if let Some(t) = app.get_webview_window("timer") {
        let _ = t.hide();
    }
}

/// Il planner (finestra main) è a schermo?
fn main_visible(app: &tauri::AppHandle) -> bool {
    app.get_webview_window("main").and_then(|w| w.is_visible().ok()).unwrap_or(true)
}

/// Posiziona la finestrella in alto a destra: un filo più in basso se fissata,
/// attaccata alla barra dei menu se non fissata.
fn place_timer(win: &tauri::WebviewWindow) {
    if let Ok(Some(mon)) = win.primary_monitor() {
        let sz = mon.size();
        let sf = mon.scale_factor();
        let lw = sz.width as f64 / sf;
        // il box ha 16px di margine sopra: alzo la finestra così il box resta
        // dove stava; il margine trasparente attorno serve a NON tagliare l'ombra
        let y = if PINNED.load(Ordering::SeqCst) { 24.0 } else { 12.0 };
        let _ = win.set_position(tauri::LogicalPosition::new(lw - 334.0 + 6.0, y));
    }
}

/// Mostra la finestrella-timer staccata, applicando lo stato pin.
/// IMPORTANTE: quando è FISSATA non prende il focus (non attiva l'app), così
/// puoi continuare a scrivere nelle altre app; galleggia solo sopra.
fn show_timer(app: &tauri::AppHandle) {
    // se il planner è a schermo, niente box (la barra in basso basta)
    if main_visible(app) {
        return;
    }
    if let Some(w) = app.get_webview_window("timer") {
        let pinned = PINNED.load(Ordering::SeqCst);
        let _ = w.set_always_on_top(pinned);
        place_timer(&w);
        let already = w.is_visible().unwrap_or(false);
        if !already {
            let _ = w.show();
        }
        // Il focus serve SOLO al box non-fissato (per nascondersi al blur). Il box
        // fissato non va mai messo a fuoco, altrimenti ruba l'attivazione.
        if !pinned && !already {
            let _ = w.set_focus();
            SUPPRESS_BLUR.store(true, Ordering::SeqCst);
            std::thread::spawn(|| {
                std::thread::sleep(std::time::Duration::from_millis(600));
                SUPPRESS_BLUR.store(false, Ordering::SeqCst);
            });
        }
    }
}

/// Mostra il box in modalità NOTIFICA (promemoria): appare in alto a destra
/// SEMPRE, anche se il planner è aperto — è una notifica, deve galleggiare
/// sopra tutto. Non ruba il focus (così puoi continuare a lavorare).
fn show_notif_box(app: &tauri::AppHandle, payload: &str) {
    NOTIF_ACTIVE.store(true, Ordering::SeqCst);
    if let Some(w) = app.get_webview_window("timer") {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(payload) {
            let _ = w.emit_to("timer", "pt-notify", v);
        }
        place_timer(&w);
        let _ = w.set_always_on_top(true);
        if !w.is_visible().unwrap_or(false) {
            let _ = w.show();
        }
    }
}

/// Clic sull'icona del tray:
/// - se NON c'è un timer attivo → apre direttamente il planner (niente box vuoto);
/// - se c'è un timer attivo → interruttore sul box (aperto lo chiude, chiuso lo apre).
fn toggle_timer(app: &tauri::AppHandle) {
    if !WAS_ACTIVE.load(Ordering::SeqCst) {
        show_main(app);
        return;
    }
    if let Some(w) = app.get_webview_window("timer") {
        if w.is_visible().unwrap_or(false) {
            let _ = w.hide();
        } else {
            show_timer(app);
        }
    }
}

/// Applica gli aggiornamenti richiesti dal planner (evento "pt-app"):
/// { "title": "01:59", "visible": true, "badge": 3 }. Il countdown nel tray si
/// mostra SOLO quando la finestrella-timer è chiusa (icona e timer si scambiano).
fn apply_pt(app: &tauri::AppHandle, payload: &str) {
    let v: serde_json::Value = match serde_json::from_str(payload) {
        Ok(v) => v,
        Err(_) => return,
    };
    let box_vis = app
        .get_webview_window("timer")
        .and_then(|w| w.is_visible().ok())
        .unwrap_or(false);
    if let Some(tray) = app.tray_by_id("tray") {
        if let Some(t) = v.get("title") {
            let title = t.as_str().unwrap_or("");
            let show = if box_vis || title.is_empty() { None } else { Some(title.to_string()) };
            let _ = tray.set_title(show);
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
/// browser". Per gli URL app.clickup.com prova prima il deep-link clickup://;
/// se nessun gestore è registrato, ripiega sull'URL https nel browser.
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

            // Finestrella-timer staccata (senza bordi, trasparente). Di default
            // NON fissata: vive sotto la barra dei menu e si nasconde su blur.
            let timer_win = WebviewWindowBuilder::new(app, "timer", WebviewUrl::App("timer.html".into()))
                .title("PIM Tomato Timer")
                .inner_size(334.0, 192.0)
                .resizable(false)
                .decorations(false)
                .transparent(true)
                .always_on_top(false)
                .shadow(false)
                .skip_taskbar(true)
                .visible(false)
                .build()?;
            place_timer(&timer_win);
            // Non fissata + focus perso → si nasconde (torna nel tray il countdown).
            let tw_blur = timer_win.clone();
            timer_win.on_window_event(move |event| {
                if let tauri::WindowEvent::Focused(false) = event {
                    if !PINNED.load(Ordering::SeqCst) && !SUPPRESS_BLUR.load(Ordering::SeqCst) {
                        let _ = tw_blur.hide();
                    }
                }
            });

            // Menù del tray (clic destro)
            let open_i = MenuItem::with_id(app, "open", "Apri PIM Tomato", true, None::<&str>)?;
            let timer_i = MenuItem::with_id(app, "timer", "Mostra timer", true, None::<&str>)?;
            let quit_i = MenuItem::with_id(app, "quit", "Esci", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&open_i, &timer_i, &quit_i])?;

            // Icona 🍅 MONOCROMATICA (template). Clic SINISTRO = mostra il box timer.
            let tray_icon = Image::from_bytes(include_bytes!("../icons/tray.png"))?;
            let _tray = TrayIconBuilder::with_id("tray")
                .icon(tray_icon)
                .icon_as_template(true)
                .tooltip("PIM Tomato")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "open" => show_main(app),
                    "timer" => show_timer(app),
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                        toggle_timer(tray.app_handle());
                    }
                })
                .build(app)?;

            // Aggiornamenti dal planner: countdown nel tray / icona / badge Dock
            let handle = app.handle().clone();
            app.listen("pt-app", move |event| {
                apply_pt(&handle, event.payload());
            });

            // Apertura link esterni (es. "apri in ClickUp") fuori dalla finestra.
            app.listen("pt-open", move |event| {
                open_external(event.payload());
            });

            // Notifiche (promemoria di fine blocco / pausetta): il planner le manda
            // qui e compaiono nel box in alto a destra, fuori dall'app.
            let h_notif = app.handle().clone();
            app.listen("pt-notif", move |event| {
                show_notif_box(&h_notif, event.payload());
            });

            // PONTE planner → finestrella-timer. NB: l'evento RILANCIATO ha nome
            // DIVERSO (pt-state) da quello ascoltato (pt-timer), altrimenti loop
            // infinito. La finestrella appare SOLO quando la giornata parte
            // (fronte attivo), non a ogni tick.
            let h_timer = app.handle().clone();
            app.listen("pt-timer", move |event| {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(event.payload()) {
                    let active = v.get("active").and_then(|x| x.as_bool()).unwrap_or(false);
                    if active {
                        if !WAS_ACTIVE.swap(true, Ordering::SeqCst) {
                            show_timer(&h_timer);
                        }
                    } else {
                        WAS_ACTIVE.store(false, Ordering::SeqCst);
                        // niente giornata attiva: via il countdown dal tray e nascondi il box
                        if let Some(tray) = h_timer.tray_by_id("tray") { let _ = tray.set_title(None::<String>); }
                        // MA non nasconderlo se sta mostrando una notifica: resta finché "Ho capito"
                        if !NOTIF_ACTIVE.load(Ordering::SeqCst) {
                            if let Some(w) = h_timer.get_webview_window("timer") { let _ = w.hide(); }
                        }
                    }
                    let _ = h_timer.emit_to("timer", "pt-state", v);
                }
            });
            // PONTE finestrella → planner: hide/pin gestiti qui; finito/pausa
            // rilanciati al planner come "pt-cmd".
            let h_act = app.handle().clone();
            app.listen("pt-timer-action", move |event| {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(event.payload()) {
                    match v.get("a").and_then(|x| x.as_str()) {
                        Some("resize") => {
                            // la finestrella si adatta all'altezza reale del box
                            // (misurata dal layout) così l'ombra non viene mai tagliata
                            if let (Some(w), Some(hh)) = (
                                h_act.get_webview_window("timer"),
                                v.get("h").and_then(|x| x.as_f64()),
                            ) {
                                let _ = w.set_size(tauri::LogicalSize::new(334.0, hh));
                                place_timer(&w);
                            }
                        }
                        Some("notif-close") => {
                            // "Ho capito": chiudo la notifica. Ripristino l'always-on-top
                            // allo stato pin e nascondo il box se il planner è aperto o
                            // non c'è più un timer attivo; altrimenti torna a mostrare il timer.
                            NOTIF_ACTIVE.store(false, Ordering::SeqCst);
                            if let Some(w) = h_act.get_webview_window("timer") {
                                let _ = w.set_always_on_top(PINNED.load(Ordering::SeqCst));
                                if main_visible(&h_act) || !WAS_ACTIVE.load(Ordering::SeqCst) {
                                    let _ = w.hide();
                                }
                            }
                        }
                        Some("hide") => {
                            if let Some(w) = h_act.get_webview_window("timer") { let _ = w.hide(); }
                        }
                        Some("open") => {
                            show_main(&h_act); // freccia: apri il planner e chiudi il box
                        }
                        Some("pin") => {
                            let on = v.get("on").and_then(|x| x.as_bool()).unwrap_or(false);
                            PINNED.store(on, Ordering::SeqCst);
                            if let Some(w) = h_act.get_webview_window("timer") {
                                let _ = w.set_always_on_top(on);
                                place_timer(&w);
                                if !w.is_visible().unwrap_or(false) { let _ = w.show(); }
                                // niente set_focus: il box fissato non ruba l'attivazione
                            }
                        }
                        _ => {}
                    }
                    let _ = h_act.emit_to("main", "pt-cmd", v);
                }
            });

            // La X rossa del planner NON chiude l'app: nasconde solo la finestra.
            if let Some(win) = app.get_webview_window("main") {
                let w = win.clone();
                let h_close = app.handle().clone();
                win.on_window_event(move |event| {
                    if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                        api.prevent_close();
                        let _ = w.hide();
                        // planner chiuso: se c'è una giornata attiva, mostra il box a destra
                        if WAS_ACTIVE.load(Ordering::SeqCst) {
                            show_timer(&h_close);
                        }
                    }
                });
            }

            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("errore nell'avvio di PIM Tomato")
        .run(|app, event| {
            if let tauri::RunEvent::Reopen { .. } = event {
                show_main(app);
            }
        });
}
