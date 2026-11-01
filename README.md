# Optic Chat

LM Studio: choose **LM Studio** under Connection, leave the server URL at `http://127.0.0.1:1234/v1` (or enter your server’s URL), and Refresh models. Select a vision-capable model for screenshots, then Save. The token is optional unless LM Studio server authentication is enabled. The server must be enabled in LM Studio’s Developer tab. This uses LM Studio’s OpenAI-compatible Responses endpoint for streaming and image input.

Hermes subscription connection: select **Hermes** in Connection to use the existing Hermes ChatGPT/Codex login. Optic Chat connects to an authenticated loopback-only bridge at `127.0.0.1:8642`. The bridge uses the installed Hermes runtime with tools, memory and background review disabled; OAuth credentials remain managed by Hermes. It starts at login through `optic-chat-hermes.service`. Restart it with `systemctl --user restart optic-chat-hermes`. Model availability and account limits are those of the Hermes login. OpenAI and Anthropic API-key connections remain selectable.

A small Rust + Slint desktop coach for GNOME Linux. A floating input bar asks OpenAI about recent screen activity; reply text appears directly below it with an adjustable transparent background. The same window transforms into settings when you press the gear.

## Start

```sh
./run.sh
```

The first launch builds the release binary. Rust can be installed from https://rustup.rs. Runtime packages: an XWayland/X11 session, OpenGL, a working desktop ScreenCast portal, PipeWire, and GStreamer with `pipewiresrc`, `videorate`, `videoconvert`, `videoscale`, `jpegenc` and `fdsink`. On Ubuntu these normally come from `gstreamer1.0-tools gstreamer1.0-pipewire gstreamer1.0-plugins-base gstreamer1.0-plugins-good xwayland`.

1. Open the gear → Connection. Enter an OpenAI API key, wait for the available model list, search or click a vision-capable model, and Save. Models load automatically after pasting a key; Refresh models retries the request. `OPENAI_API_KEY` also works and takes precedence. Keys are stored in the Linux secret service, never settings JSON.
2. Click Share. Approve a monitor or a specific window in the desktop picker. Window sharing avoids including the floating chat itself. Capture is off at startup.
3. Ask a question. The latest five distinct retained frames are attached, oldest first, with their ages. Text replies stream into the bubble. You can still ask text-only questions without sharing.
4. In Appearance, choose a theme, click color swatches, or enter custom hex colors. The reply-background opacity slider runs from 0% (transparent) to 100% (solid), with a live preview. Save to keep changes. Reply dimensions are adjustable below the color controls.
5. Apply shortcuts in the Shortcuts tab (or run `./run.sh --install-shortcut`). On GNOME this registers the app’s global shortcuts, including **Ctrl+Super+Space**. Existing shortcut entries are preserved. On other desktops bind `.../target/release/optic-chat --toggle-bubble` in keyboard settings. The Reply button also toggles the bubble.

The settings window is 500×500 logical pixels. Chat mode adapts to the configured reply width and expands when a reply is shown. The gear fades and slides the chat into settings over 420 ms; Back restores chat. Drag the grip to move the overlay, or drag the settings heading while settings are open. Sizes follow desktop scaling.

Install an Applications launcher with `./scripts/install-desktop.sh`.

## Screen buffer

- One permissioned ScreenCast session; the local capture helper samples at 1 fps, with an adjustable 1–30 second acceptance cadence (default 2 s).
- A 64×36 RGB thumbnail detects substantial visual changes. Pixels must change by an average of at least 24 levels; the configured percentage of changed pixels is compared against **all** retained signatures. Cursor motion is excluded. Returning to an already retained scene moves it to the newest position and refreshes its timestamp.
- At most 10 distinct JPEGs are held in RAM; frame 11 evicts frame 1. Each question snapshots the newest five. There are no screenshot files or unlimited history. Images in an in-flight request are released when it finishes.
- Default maximum width is 1280 px, JPEG quality 60. A hard 1 MB per-image limit bounds retained image storage to 10 MB. Defaults are generally much smaller but image complexity affects size.
- OpenAI image inputs use `detail: low`. JPEG compression reduces upload bytes; low detail reduces image processing context. Small text may be illegible. Increase resolution/quality if necessary, but low detail still limits fine detail.
- Pause stops capture but keeps the buffer; Clear deletes the retained images. Old images are age-labeled. Quit releases all frames.
- Every request uploads only its selected images and text to OpenAI. `store: false` is sent, and only six text exchanges are replayed. API-provider retention policies still apply. No screenshot uploads happen just from sharing a screen.

## Linux behavior

The Slint app is built with the X11 backend and runs under XWayland on GNOME Wayland. This is intentional: GNOME can honor its always-on-top request there. Screen capture still uses the Wayland portal. A compositor may reserve fullscreen/system-modal surfaces above normal pinned windows. The unified overlay is frameless. Alt+F4 closes it. No background service is installed.

The GNOME shortcut launches the existing binary with `--toggle-bubble`, which sends a command through a private user-only Unix socket. `--status` reports buffer counts/bytes for diagnostics, without image content or credentials.

To remove the GNOME shortcut, remove “Optic Chat · toggle response” in GNOME Settings → Keyboard → Custom Shortcuts. If you move or rebuild the executable at another path, re-enable the shortcut.

## Development

```sh
./scripts/build.sh check
./scripts/build.sh test
./scripts/build.sh build
```

Source modules: `config.rs` (validated settings/keyring), `buffer.rs` (deduplication/compression/eviction), `capture.rs` (portal + system GStreamer helper), `api.rs` (OpenAI request/stream parsing), `desktop.rs` (local IPC/shortcut), and `ui/app.slint` (native UI).

## Voice input

Click the microphone or press **Ctrl+Super+M** to start recording from the system's default microphone. Press again to stop. Local Whisper transcription fills the question field; review it and press Send. Recording stops automatically after two minutes. Audio is deleted after transcription; the local worker exits and releases its memory. No cloud transcription key is needed.

Change the microphone shortcut in Settings → Shortcuts. Set the default microphone in Linux Sound settings. For another installation, install PipeWire's `pw-record` and run `bash scripts/setup-voice.sh` (requires `uv`; downloads the Whisper base model on first setup).

## Floating layout update

The working view is now a single frameless window: a rounded input bar, small controls, and reply text on an adjustable transparent background below. The gear transforms that same window into the 500×500 settings view with a 420 ms fade/slide animation inside a stable native window; Back returns to chat. Drag the grip at the left of the input bar to move the whole overlay. Ctrl+Super+Space hides/shows only the reply. Alt+F4 quits. Reply background opacity is adjustable in Appearance. Input wraps and grows up to three rows, then scrolls internally. Enter sends; Shift+Enter adds a line. The overlay stays within the current monitor’s usable work area when dragged, resized, or expanded, including monitors with negative coordinates.

Settings use full-width Connection, Appearance, and Screen tabs. Screen numeric fields share one width; every settings page includes bottom padding so its last action can scroll fully into view. Reply scrolling includes the last line and bottom padding. The adjustable reply backdrop extends behind the input bar, with matching outer edges.

## Providers and shortcuts

Connection supports OpenAI and Anthropic. Select the provider, paste its API key, choose a model from the fetched list, then Save. Keys are stored separately in the Linux keyring. `OPENAI_API_KEY` and `ANTHROPIC_API_KEY` override the matching saved keys. Anthropic uses its [Messages API](https://platform.claude.com/docs/en/api/messages/create) for streamed text and JPEG screenshot input. Each provider remembers its chosen model.

The settings header includes Quit to fully exit. The Shortcuts tab supports recording combinations, manual accelerator editing, and clearing bindings. Save settings registers changes with GNOME; Apply shortcuts registers the current fields immediately (Save also retains them in app settings). Existing unrelated GNOME shortcuts are preserved.

Default global keys:
- Ctrl+Super+H: show/hide the app; launches it if closed.
- Ctrl+Super+Space: show/hide the reply.
- Ctrl+Super+Left: move left, keeping vertical position.
- Ctrl+Super+Up: move up, keeping horizontal position.
- Ctrl+Super+Down: move down, keeping horizontal position.
- Ctrl+Super+Right: move right, keeping vertical position.

Corner moves respect the current monitor’s usable work area. Quit is separate from the show/hide toggle.

Corner moves animate for 320 ms with easing. New corner commands redirect from the current position; dragging cancels the movement. Fn is not exposed as a modifier by this laptop’s keyboard, so the corner defaults use Ctrl+Super+arrows.

Arrow shortcuts now move along one axis: Ctrl+Super+Left/Right preserve vertical position; Ctrl+Super+Up/Down preserve horizontal position. Quick successive arrows combine their destinations during animation.

Ctrl+Super+I shows the app and focuses the question input, including when settings are open or the app is hidden. Edit or record this binding under Shortcuts → Focus text input.

Window placement uses GNOME's per-monitor GTK work areas when available, falling back to the desktop work area. This reserves actual panels and docks without hardcoding a Linux top-bar height. There is no additional outer margin; the existing 8px UI inset supplies the visible edge gap.
