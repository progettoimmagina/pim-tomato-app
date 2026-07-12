# ▶️ PIM Tomato — App Mac (Tauri)

App Mac nativa che fa da **guscio** al planner PIM Tomato (WordPress su studio.progettoimmagina.com). Uso interno PIM. Deciso 2026-07-10, scaffold + prima build v0.1.0 lo stesso giorno.

## Stato (v0.1.0 — 2026-07-10)
- Finestra nativa che carica `https://studio.progettoimmagina.com/planner/` dal vivo (risultata già loggata al primo test → cookie persistono nel webview).
- **🍅 nella barra dei menu** (tray, in alto a destra) con menù *Apri PIM Tomato* / *Esci* — VERIFICATO a schermo.
- **Finestra SENZA barra grigia** (titleBarStyle "Overlay" + hiddenTitle) → look "app vera"; il planner (v0.6.6) aggiunge padding-left al topbar via classe `pt-app` per non finire sotto i pallini semaforo.
- **Icona app = cyan + pomodoro grigio scuro** (stile slide Blog PIM social kit; `app-icon.png` generato con `scratchpad/mkicon.py`). Sostituisce il pomodoro rosso precedente.
- **NOTIFICHE NATIVE** dentro l'app: withGlobalTauri:true + capabilities `remote.urls`=["https://studio.progettoimmagina.com/*"] + permessi notification. Il planner (v0.6.6) rileva Tauri (`window.__TAURI__.core`, var INAPP) e usa `invoke('plugin:notification|notify' / 'request_permission' / 'is_permission_granted')` al posto del web push (campanella, avvia giornata, avvisi 5min/fine blocco). **VERIFICATO A SCHERMO (2026-07-10):** clic campanella → banner notifica PIM Tomato comparso (rilevato "Centro Notifiche" sopra la finestra) → il pattern remote.urls FUNZIONA così com'è.
- **TRASCINAMENTO finestra** (v0.6.7): la topbar ha `data-tauri-drag-region` (+ brand-mark/name) e le capabilities hanno `core:window:allow-start-dragging`. VERIFICATO: trascinando la barra la finestra si sposta.
- Build OK (build incrementali ~11s): `PIM Tomato.app` + `PIM Tomato_0.1.0_aarch64.dmg` in `src-tauri/target/release/bundle/`. Copia in root: `PIM Tomato 0.1.0.dmg` (aggiornata).
- **TUTTO VERIFICATO A SCHERMO 2026-07-10:** finestra senza barra grigia + drag OK + icona cyan (anche nel tray della barra dei menu) + notifiche native OK.
**RIFINITURE 2026-07-11 (plugin fino a v0.7.0):**
- **ICONA rifatta** (app-icon.png via scratchpad/mkicon2.py): corpo pomodoro squat grigio scuro + highlight + **calice a FOGLIE grigio chiaro + stelo** (prima era una stella, si leggeva male). Verificata nel Dock, ora si legge come pomodoro. (Se ancora non piace: prossima opzione calice VERDE.)
- **SOVRAPPOSIZIONE barra corretta su TUTTE le pagine** (prima solo "le mie task"): rilevamento Tauri spostato in `PIMPL_Render::enqueue_assets` (inline su ogni pagina con JS) + regola `.pt-app .topbar{padding-left:120px}` in fonts.css (condiviso). griglia+impostazioni ora hanno il padding.
- **DRAG senza bloccare i pulsanti** (v0.7.0): la topbar-intera aveva `data-tauri-drag-region` e ingoiava i clic → ora una `<span class="topdrag" data-tauri-drag-region>` (flex:1) SOLO al centro, aggiunta ai 3 template (collaboratore/griglia/impostazioni); topbar+brand senza attributo → pulsanti cliccabili, drag dal centro.
**BARRA DEI MENU + BADGE (2026-07-11, plugin v0.7.4/0.7.5 + rebuild app):**
- Tray icona MONOCROMATICA (template): src-tauri/icons/tray.png = silhouette pomodoro nera (scratchpad/mktray.py), `.icon_as_template(true)`. Cargo feature `image-png` per `Image::from_bytes`.
- **TIMER nel tray** accanto all'icona quando la giornata è attiva (countdown del blocco corrente, formato H:MM o M:SS) — VERIFICATO a schermo ("🍅 1:39").
- **BADGE nel Dock** = n° blocchi da fare (window.set_badge_count) — VERIFICATO ("2").
- **Toggle in Impostazioni** ("App Mac"): icona barra menu on/off + badge on/off (localStorage pt_tray/pt_badge, per-macchina).
- ⚠️ **I COMANDI CUSTOM NON PASSANO DA REMOTO** (set_tray_title ecc. bloccati): risolto con EVENTI Tauri. Il planner emette `window.__TAURI__.event.emit('pt-app', {title,visible,badge})`; Rust `app.listen("pt-app")` → apply_pt() aggiorna tray/badge. Capability: `core:event:allow-emit` + `allow-emit-to`. QUESTO È IL PATTERN per far comunicare planner→guscio (non usare invoke di comandi custom da remoto; usare eventi con permesso core:event, o plugin con permessi).

**RITOCCHI 2026-07-11 (icona v3 + finestra + Cmd+R):**
- **ICONA v3**: bg NERO (#1a1917 rounded-rect) + **pomodoro CYAN fuso con TIMER** — corpo cyan + calice/stelo cyan + lancette orologio (colore bg, "~10:10") + perno; leggibile in piccolo, niente numeri. Script scratchpad/mkicon3.py → app-icon.png → tauri icon. (Il tray in barra menu resta la silhouette monocromatica separata tray.png.)
- **FINESTRA di default più grande**: tauri.conf.json width 1400 height 920 + center:true (era 1120×820).
- **Cmd+R** ricarica la pagina: handler keydown nell'inline condiviso (render.php enqueue_assets, solo se window.__TAURI__ → location.reload()). Plugin v0.7.6. NB: non testabile via computer-use (i tasti nella webview dell'app non registrano affidabilmente + focus ruba da "Claude"); confermato solo che il codice è live.
- ⚠️ Il bundle .dmg a volte si impicca su bundle_dmg.sh (volume montato): `hdiutil detach /Volumes/PIM\ Tomato* -force` + il `.app` è comunque già pronto in target/release/bundle/macos/.

**⚠️ LIMITE TOOLING:** i clic di computer-use NON registrano nella webview WKWebView dell'app (nemmeno la barra ricerca si apriva); il drag della finestra sì. Quindi il test di pulsanti/nav/drag va fatto da Niccolò coi clic reali (che funzionano). Icona e look verificabili via screenshot; interazioni no.

## Architettura (thin shell — NON riscrittura)
- Il backend/cervello resta WordPress (plugin pim-planner). L'app mostra il planner dal vivo → **le modifiche UI del plugin appaiono nell'app senza ricostruirla**.
- La build dell'app serve solo quando cambia la parte NATIVA (tray, notifiche, avvio al login).

## Toolchain (installato 2026-07-10)
- Rust 1.97 (rustup, `~/.cargo`). Node 24, Xcode CLI, Homebrew già presenti.
- Build: `source ~/.cargo/env && npx tauri build` (prima build ~qualche min; poi incrementale veloce).
- Icone: `npx tauri icon app-icon.png`.

## Struttura
- `src-tauri/` (Cargo.toml, tauri.conf.json, src/lib.rs = tray+setup, src/main.rs, capabilities/default.json, icons/).
- `dist/index.html` = splash (mostrato solo se il remoto non carica).
- `package.json` (solo @tauri-apps/cli).

## PROSSIMI PASSI
1. **Firma/notarizzazione**: per ora NON firmata → prima apertura tasto destro→Apri (Gatekeeper). Firma Apple Developer 99$/anno solo se serve zero-avvisi per il team.
2. **Repo GitHub + auto-update Tauri**: creare repo (Niccolò clicca), configurare l'updater Tauri (chiave updater + endpoint releases) → l'app si aggiorna da sola. Serve `tauri-plugin-updater` + firma updater.
3. **Countdown nella barra dei menu** (il pezzo forte): la pagina planner, se dentro Tauri (`window.__TAURI__`), emette ogni secondo il tempo rimanente del blocco corrente → comando Rust che aggiorna il titolo del tray (es. "🍅 01:59"). Serve `dangerousRemoteDomainIpcAccess` per studio.progettoimmagina.com nelle capabilities.
4. Aprire link esterni (es. "apri in ClickUp") nel browser di sistema, non dentro l'app.
5. Versionamento con release.sh dedicato (o script Tauri) quando parte l'auto-update.

## AUTO-UPDATE (2026-07-12, app v0.2.0)
- Plugin `tauri-plugin-updater` aggiunto; controllo all'avvio in `lib.rs` (check→download→install→restart, silenzioso).
- `tauri.conf.json`: `bundle.createUpdaterArtifacts:true` + blocco `plugins.updater` (endpoint `https://github.com/progettoimmagina/pim-tomato-app/releases/latest/download/latest.json` + pubkey).
- Chiave di firma updater: `~/.tauri/pimtomato_updater.key` (+ `.pub`); backup nel **Keychain** (`pimtomato-updater` / `pimtomato-updater-key`). SENZA password.
- Link esterni: evento `pt-open` → `open_external()` in Rust apre ClickUp (deep-link `clickup://`) se installata, altrimenti browser. Il planner (v0.7.11) emette `pt-open` per ogni link esterno (delegato click in `collaboratore.js`).
- Build firmata: `TAURI_SIGNING_PRIVATE_KEY=$(cat ~/.tauri/pimtomato_updater.key) TAURI_SIGNING_PRIVATE_KEY_PASSWORD="" npx tauri build` → produce `PIM Tomato.app.tar.gz` + `.sig` + dmg.
- Release script: `build-tools/app-release.sh <ver> "titolo"` (bump→build firmata→latest.json→push→gh release).
- Artefatti v0.2.0 già pronti in `dist-release/` + `latest.json`. Installer: `PIM Tomato 0.2.0.dmg` (root).
- ⏳ **BLOCCO**: il PAT `gh` NON può creare repo (`createRepository` negato). Serve creare a mano il repo **pubblico** `progettoimmagina/pim-tomato-app` (vuoto, no README). Poi: `git remote add origin … && git push -u origin main --tags` + `gh release create v0.2.0 …` (o rilanciare app-release.sh). Endpoint updater legge l'ultima release.

## Comando per ricostruire
```
cd "/Users/niccolofalaschi/Documents/Claude app/pim-tomato-app" && source "$HOME/.cargo/env" && npx tauri build
```
