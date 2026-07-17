use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
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
/// Preferenza suono notifiche (per-macchina, inviata dal planner via pt-app).
static SOUND_ON: AtomicBool = AtomicBool::new(true);
static SOUND_NAME: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());

/// Conto alla rovescia del tray gestito DALL'APP (1s nativo), non dal JS del
/// planner: quando il planner è in secondo piano il suo timer è throttlato e il
/// tray saltava di 2s. Statici alimentati dagli eventi pt-timer.
static TRAY_END_MS: AtomicI64 = AtomicI64::new(0); // fine blocco corrente (epoch ms)
static TRAY_PAUSED: AtomicBool = AtomicBool::new(false);
static TRAY_PAUSED_REM: AtomicI64 = AtomicI64::new(0); // secondi rimasti se in pausa
static TRAY_ON: AtomicBool = AtomicBool::new(true); // preferenza "mostra conto nel menu"

/// Formatta i secondi come il planner: h:mm:ss oppure m:ss.
fn fmt_tray(sec: i64) -> String {
    let s = sec.max(0);
    let (h, m, x) = (s / 3600, (s % 3600) / 60, s % 60);
    if h > 0 {
        format!("{}:{:02}:{:02}", h, m, x)
    } else {
        format!("{}:{:02}", m, x)
    }
}

/// Suona il suono di notifica scelto (macOS, suoni di sistema).
fn play_notif_sound(force: bool) {
    if !force && !SOUND_ON.load(Ordering::SeqCst) {
        return;
    }
    let name = SOUND_NAME.lock().ok().map(|g| g.clone()).unwrap_or_default();
    let name = if name.is_empty() { "Glass".to_string() } else { name };
    let _ = std::process::Command::new("afplay")
        .arg(format!("/System/Library/Sounds/{}.aiff", name))
        .spawn();
}

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
        // dove stava; il margine trasparente attorno serve a NON tagliare l'ombra.
        // Spostato a SINISTRA della corsia delle notifiche macOS (~380px, non
        // riposizionabili da app): il box resta in alto, i banner Mac scivolano
        // alla sua destra, MAI sovrapposti (richiesta Niccolò 2026-07-16).
        let y = if PINNED.load(Ordering::SeqCst) { 24.0 } else { 12.0 };
        let _ = win.set_position(tauri::LogicalPosition::new(lw - 334.0 + 6.0 - 380.0, y));
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
    // suono di notifica (macOS): affidabile anche a finestra nascosta,
    // indipendente dallo stato audio del webview; rispetta la preferenza utente
    play_notif_sound(false);
    if let Some(w) = app.get_webview_window("timer") {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(payload) {
            let _ = w.emit_to("timer", "pt-notify", v);
        }
        place_timer(&w);
        let _ = w.set_always_on_top(true);
        if !w.is_visible().unwrap_or(false) {
            let _ = w.show();
        }
        // Una NOTIFICA vuole una decisione: le do il focus così il PRIMO clic su
        // un bottone compie l'azione (prima il primo clic serviva solo ad
        // attivare la finestra e ne serviva un secondo). SUPPRESS_BLUR evita che
        // il giro focus/blur la richiuda subito.
        SUPPRESS_BLUR.store(true, Ordering::SeqCst);
        let _ = w.set_focus();
        let h_sb = app.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(400));
            SUPPRESS_BLUR.store(false, Ordering::SeqCst);
            let _ = h_sb; // handle tenuto vivo per il thread
        });
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
    /* preferenza suono notifiche (on/off + quale suono di sistema) */
    if let Some(s) = v.get("sound").and_then(|x| x.as_bool()) {
        SOUND_ON.store(s, Ordering::SeqCst);
    }
    if let Some(n) = v.get("soundName").and_then(|x| x.as_str()) {
        let clean: String = n.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
        if let Ok(mut g) = SOUND_NAME.lock() {
            *g = clean;
        }
    }
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

            // Aggiornamenti MENTRE l'app è aperta: controllo periodico (ogni 2 ore).
            // Se c'è una versione nuova NON installo in silenzio: avviso il planner
            // (evento "pt-update") che mostra il modale "aggiorna ↻".
            let up_loop = app.handle().clone();
            std::thread::spawn(move || loop {
                std::thread::sleep(std::time::Duration::from_secs(2 * 3600));
                let h = up_loop.clone();
                tauri::async_runtime::spawn(async move {
                    use tauri_plugin_updater::UpdaterExt;
                    if let Ok(updater) = h.updater() {
                        if let Ok(Some(update)) = updater.check().await {
                            let _ = h.emit_to("main", "pt-update", serde_json::json!({ "version": update.version }));
                        }
                    }
                });
            });

            // Conto alla rovescia del tray calcolato DALL'APP ogni secondo: quando
            // il planner è in secondo piano il suo timer JS è throttlato e il tray
            // saltava di 2s. Qui è sempre a 1s (nativo), coerente col box.
            let tray_loop = app.handle().clone();
            std::thread::spawn(move || loop {
                std::thread::sleep(std::time::Duration::from_secs(1));
                if !WAS_ACTIVE.load(Ordering::SeqCst) || !TRAY_ON.load(Ordering::SeqCst) {
                    continue;
                }
                let h = tray_loop.clone();
                let _ = h.clone().run_on_main_thread(move || {
                    if !WAS_ACTIVE.load(Ordering::SeqCst) || !TRAY_ON.load(Ordering::SeqCst) {
                        return;
                    }
                    // box visibile → il conto lo mostra il box, non il tray
                    let box_vis = h
                        .get_webview_window("timer")
                        .and_then(|w| w.is_visible().ok())
                        .unwrap_or(false);
                    if box_vis {
                        return;
                    }
                    let sec = if TRAY_PAUSED.load(Ordering::SeqCst) {
                        TRAY_PAUSED_REM.load(Ordering::SeqCst)
                    } else {
                        let now_ms = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_millis() as i64)
                            .unwrap_or(0);
                        (TRAY_END_MS.load(Ordering::SeqCst) - now_ms) / 1000
                    };
                    if let Some(tray) = h.tray_by_id("tray") {
                        let _ = tray.set_title(Some(fmt_tray(sec)));
                    }
                });
            });

            // Clic su "aggiorna" nel modale → scarica, installa e riavvia da solo.
            let h_upgo = app.handle().clone();
            app.listen("pt-update-go", move |_event| {
                let h = h_upgo.clone();
                tauri::async_runtime::spawn(async move {
                    use tauri_plugin_updater::UpdaterExt;
                    if let Ok(updater) = h.updater() {
                        if let Ok(Some(update)) = updater.check().await {
                            if update.download_and_install(|_, _| {}, || {}).await.is_ok() {
                                h.restart();
                            }
                        }
                    }
                    /* se siamo ancora qui, qualcosa è andato storto */
                    let _ = h.emit_to("main", "pt-update-fail", serde_json::json!({}));
                });
            });

            // ── SPLASH di avvio: la finestra principale parte NASCOSTA (visible:false).
            // Mostro prima una schermata locale (bg dark SUBITO → niente flash bianco),
            // con logo SolitonAI + i messaggi. Il planner, appena carica, emette
            // "planner-ready"; la splash saluta e poi emette "splash-done" → apro l'app.
            let _splash = WebviewWindowBuilder::new(app, "splash", WebviewUrl::App("splash.html".into()))
                .title("PIM Tomato")
                .inner_size(1400.0, 920.0)
                .min_inner_size(760.0, 560.0)
                .center()
                .title_bar_style(tauri::TitleBarStyle::Overlay)
                .hidden_title(true)
                .build()?;

            let h_done = app.handle().clone();
            app.listen("splash-done", move |_event| {
                if let Some(m) = h_done.get_webview_window("main") { let _ = m.show(); let _ = m.set_focus(); }
                if let Some(s) = h_done.get_webview_window("splash") { let _ = s.close(); }
            });
            // rete di sicurezza: se qualcosa va storto, apri comunque dopo 12s
            let h_fb = app.handle().clone();
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_secs(12));
                if h_fb.get_webview_window("splash").is_some() {
                    if let Some(m) = h_fb.get_webview_window("main") { let _ = m.show(); let _ = m.set_focus(); }
                    if let Some(s) = h_fb.get_webview_window("splash") { let _ = s.close(); }
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
                    // una NOTIFICA resta FISSA finché non ci clicchi qualcosa:
                    // non si nasconde cliccando altrove (richiesta Niccolò).
                    if !PINNED.load(Ordering::SeqCst)
                        && !SUPPRESS_BLUR.load(Ordering::SeqCst)
                        && !NOTIF_ACTIVE.load(Ordering::SeqCst)
                    {
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

            // Anteprima suono dalle Impostazioni ("clicca un nome per provarlo")
            app.listen("pt-sound-test", move |_event| {
                play_notif_sound(true);
            });

            // PONTE planner → finestrella-timer. NB: l'evento RILANCIATO ha nome
            // DIVERSO (pt-state) da quello ascoltato (pt-timer), altrimenti loop
            // infinito. La finestrella appare SOLO quando la giornata parte
            // (fronte attivo), non a ogni tick.
            let h_timer = app.handle().clone();
            app.listen("pt-timer", move |event| {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(event.payload()) {
                    let active = v.get("active").and_then(|x| x.as_bool()).unwrap_or(false);
                    // stato per il conto alla rovescia del tray gestito dall'app (1s)
                    if active {
                        let end_ts = v.get("endTs").and_then(|x| x.as_f64()).unwrap_or(0.0);
                        let rem = v.get("remaining").and_then(|x| x.as_f64()).unwrap_or(0.0);
                        let paused = v.get("paused").and_then(|x| x.as_bool()).unwrap_or(false);
                        let tray_on = v.get("tray").and_then(|x| x.as_bool()).unwrap_or(true);
                        TRAY_END_MS.store((end_ts * 1000.0) as i64, Ordering::SeqCst);
                        TRAY_PAUSED.store(paused, Ordering::SeqCst);
                        TRAY_PAUSED_REM.store(rem as i64, Ordering::SeqCst);
                        TRAY_ON.store(tray_on, Ordering::SeqCst);
                    } else {
                        TRAY_ON.store(false, Ordering::SeqCst);
                    }
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
                        Some("notif-yes") => {
                            // "sì" a una domanda del box (es. "Oh caxxo!"): APRO il
                            // planner (lì compare il modale giusto) e inoltro il comando.
                            NOTIF_ACTIVE.store(false, Ordering::SeqCst);
                            if let Some(w) = h_act.get_webview_window("timer") {
                                let _ = w.set_always_on_top(PINNED.load(Ordering::SeqCst));
                            }
                            show_main(&h_act); // apre l'app e nasconde il box
                            let _ = h_act.emit_to("main", "pt-notif-yes", v.clone());
                        }
                        Some("notif-snooze") => {
                            // "rimanda": chiudo il box ora e lo ri-mostro con la STESSA
                            // notifica dopo `secs` secondi (thread che dorme; ok finché
                            // l'app resta viva — è un'app da barra dei menu).
                            NOTIF_ACTIVE.store(false, Ordering::SeqCst);
                            if let Some(w) = h_act.get_webview_window("timer") {
                                let _ = w.set_always_on_top(PINNED.load(Ordering::SeqCst));
                                if main_visible(&h_act) || !WAS_ACTIVE.load(Ordering::SeqCst) {
                                    let _ = w.hide();
                                }
                            }
                            let secs = v.get("secs").and_then(|x| x.as_f64()).unwrap_or(600.0).max(60.0) as u64;
                            let data = v.get("data").cloned().unwrap_or(serde_json::json!({}));
                            let payload = serde_json::to_string(&data).unwrap_or_else(|_| "{}".into());
                            let h_snooze = h_act.clone();
                            std::thread::spawn(move || {
                                std::thread::sleep(std::time::Duration::from_secs(secs));
                                show_notif_box(&h_snooze, &payload);
                            });
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
