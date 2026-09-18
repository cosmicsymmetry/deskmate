> **STATUS — HISTORICAL REVIEW.** Reviewed 2026-08-15; retired 2026-09-18. This
> document audited the macOS Tauri companion, whose binary and `src-tauri` source tree
> have been deleted. It is **not a security review of the current web companion or Rust
> server**. Paths and controls below are preserved as evidence of what was reviewed.
>
> Finding carry-forward: F1 (Tauri capabilities), F2 (Tauri CSP), F3 (desktop-shell
> logging), and F5 (DMG signing) ended with the deleted binary. F4 survives in part:
> `app-core` still repairs readable config files to mode `0600` and creates atomic
> replacements at `0600`; the shell-owned `0700` application-directory setup did not
> carry forward. F6's local-app exception did not carry forward as policy: production
> server data-card fetches use the server's guarded egress client. These facts do not
> extend this review's verdicts to the current server.

# V1 Security Review — Companion App

Reviewed: 2026-08-15, commit `b37a1b5a74db95e360f23340cac12c2bba610f78` plus the fixes listed below in the uncommitted review working tree.

Scope: the macOS-only 1.0.0 dogfood companion. The packaged settings frontend is local code. IPC arguments are nevertheless treated as untrusted at the Rust boundary because a compromised webview could invoke any registered custom command. Bytes received from the USB device and data returned by file/network providers are independently untrusted. The review covered all Rust sources under `companion/apps/deskmate/src-tauri/src` and `companion/crates/app-core/src`, the frontend bridge, generated Tauri ACL schema, observed config permissions, and the controller's mounted-DMG evidence.

## IPC commands

The registry contains exactly the 11 commands below (`companion/apps/deskmate/src-tauri/src/lib.rs:458`). IDs are bounded before dispatch (`companion/apps/deskmate/src-tauri/src/commands.rs:619`); configuration JSON is capped at 64 KiB before deserialization (`companion/apps/deskmate/src-tauri/src/commands.rs:604`). Untrusted device events are also length-checked, restricted to closed kind/action combinations, and matched against configured IDs before they mutate runtime state (`companion/crates/protocol/src/message.rs:460`, `companion/crates/app-core/src/runtime.rs:1983`).

| Command | Inputs and attacker influence | Effect | Verdict |
|---|---|---|---|
| `get_app_snapshot` | No caller-supplied input. The returned snapshot includes bounded USB-derived firmware/status fields and untrusted provider-derived card text, as well as the user's personal config (`commands.rs:146`). | Reads and returns backend state; no mutation. | Accept. Exposure is only to the one packaged local settings window; the CSP gives it no arbitrary network egress. USB frames are bounded to 2,048 decoded bytes and typed before projection (`protocol/src/frame.rs:3`, `protocol/src/message.rs:20`). |
| `validate_config_draft` | `draft.json` is webview/user controlled; connected-device capability bits used by validation came from untrusted USB (`commands.rs:152`). | Parses and compiles a draft and returns validation/capability issues; no persistence, provider refresh, or USB write (`commands.rs:173`). | Accept. The envelope denies unknown fields, the JSON is capped at 64 KiB, strict domain validation/compilation runs, and USB data is used only in a bitmask compatibility check. |
| `save_apply_config` | `draft.json` is webview/user controlled; USB capability bits affect the preflight (`commands.rs:188`). A compromised local renderer could name provider HTTP(S) URLs or an ICS file path; the normal UI obtains file paths from the native picker. | Validates/compiles, atomically persists config, replaces runtime configuration, starts/schedules configured providers, and may send time/config/data over USB (`commands.rs:464`). | Accept for the local-only V1 trust model. The 64 KiB cap, strict schema/domain checks, capability preflight, serialized mutation lock, bounded provider I/O, and framed protocol constrain the effects. This is intentionally the most powerful IPC command. |
| `set_pushing_paused` | Boolean from the webview (`commands.rs:197`); not remotely controlled under the release CSP. | Persists the pause preference and pauses/resumes provider/device pushing (`commands.rs:400`). | Accept. Closed boolean input and serialized persistence/runtime update. Resuming can only replay already validated state. |
| `control_pomodoro` | Webview-supplied `widget_id` plus the closed `start`/`pause`/`toggle`/`reset` enum (`commands.rs:202`). | Changes one existing pomodoro and may push its bounded fields or interrupt state to the device. | Accept. ID is nonempty and at most 32 UTF-8 bytes, runtime membership/type is checked, and action deserialization is a closed enum. |
| `refresh_provider` | Webview-supplied `widget_id` (`commands.rs:215`). | Schedules an immediate read of the already-saved provider: local ICS file or bounded HTTP(S) fetch, depending on that card's config. | Accept. ID is bounded and must name an existing provider. It cannot supply or override a URL/path; provider response size, timeout, redirects, and parsers are bounded. |
| `choose_ics_file` | No IPC argument. The returned path is user-controlled through the native OS picker, not USB/network controlled (`commands.rs:227`). | Opens a native file picker, verifies the selection is a local regular file no larger than 1 MiB, bounds the UTF-8 path, and returns the path; it does not read calendar contents in this command (`commands.rs:241`). | Accept. User-mediated file selection, extension filter, regular-file check, 1 MiB bound, and no arbitrary path argument. |
| `get_autostart_status` | No caller input (`commands.rs:270`). | Reads the OS launch-at-login state and the saved preference. | Accept. Read-only and no attacker-controlled target. |
| `set_autostart_enabled` | Boolean from the webview (`commands.rs:278`). | Enables/disables a fixed LaunchAgent for Deskmate, persists the preference, updates runtime/tray state, and rolls back OS state if persistence fails (`commands.rs:415`). | Accept. The caller cannot provide a program, arguments, path, or launch mechanism. |
| `set_settings_window_visible` | Boolean from the webview (`commands.rs:287`). The returned snapshot has the same bounded USB/provider data noted for `get_app_snapshot`. | Shows/focuses or hides only the fixed `main` window and returns state. | Accept. Closed boolean and fixed window label; no arbitrary window creation or navigation. |
| `render_card_preview` | Webview-supplied `card_id`; renderer fields may contain untrusted bounded provider data, but not USB payload text (`commands.rs:320`). | Looks up an existing card, renders its configured template with last-good fields on the single simulator thread, and returns a base64 PNG (`commands.rs:337`). | Accept. ID is bounded/membership-checked, field values are bounded upstream, template mapping is closed, and only PNG bytes return to the local window. Repeated requests are latest-wins coalesced (`src-tauri/src/preview.rs:40`). |

No device-originated string selects a filesystem path, provider URL, application, or process. USB navigation/tap/dismiss events are typed and matched to live configuration before use (`companion/crates/app-core/src/runtime.rs:1983`).

## CSP and capabilities

The configured CSP is (`companion/apps/deskmate/src-tauri/tauri.conf.json:27`):

```text
default-src 'self'; connect-src ipc: http://ipc.localhost; img-src 'self' data:; style-src 'self'; script-src 'self'; object-src 'none'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'
```

- `default-src 'self'` is the deny-by-default fallback for packaged local assets.
- `connect-src ipc: http://ipc.localhost` permits only Tauri IPC transports. It does not permit the webview to fetch configured provider URLs; those are fetched by bounded Rust providers.
- `img-src 'self' data:` is required for packaged images and the simulator's `data:image/png;base64,...` preview (`companion/apps/deskmate/src/components/DevicePreview.tsx:84`).
- `style-src 'self'` is required for the packaged stylesheet and permits no remote styles.
- `script-src 'self'` is required for the packaged application bundle and permits neither inline nor remote script.
- `object-src 'none'`, `base-uri 'none'`, `form-action 'none'`, and `frame-ancestors 'none'` close non-fallback plugin, base-URL, form-navigation/egress, and framing paths. The one application form is handled entirely in React and calls `preventDefault()` (`companion/apps/deskmate/src/components/PlaylistPanel.tsx:89`).

There is no wildcard, remote web origin, `unsafe-inline`, or `unsafe-eval`. Before this review, the four explicit non-fallback restrictions were absent; F2 added them because `default-src` alone does not restrict form destinations, base URIs, or framing.

The `main` capability is restricted to window label `main` and now grants only (`companion/apps/deskmate/src-tauri/capabilities/main.json:3`):

| Permission | Frontend use | Verdict |
|---|---|---|
| `core:event:allow-listen` | `listen("app-state", ...)` subscribes to backend snapshots (`companion/apps/deskmate/src/lib/tauri.ts:139`). | Required. |
| `core:event:allow-unlisten` | The `UnlistenFn` returned by `listen` is called during lifecycle cleanup. | Required. |

Before this review, `core:event:default` also granted frontend `emit` and `emit_to`, neither of which the frontend uses. Finding F1 removed those permissions. No dialog, autostart, window-management, filesystem, shell, or opener permission is granted to the webview. Custom Rust commands are the typed mediation layer.

## URL / app-launch policy

The Tauri shell installs only autostart, dialog, and single-instance plugins (`companion/apps/deskmate/src-tauri/Cargo.toml:24`; registration at `companion/apps/deskmate/src-tauri/src/lib.rs:448`). There is no shell or opener dependency/plugin, and a source grep found no `Command::new`, `std::process`/`tokio::process` spawning API, opener call, or URL-open call. The sole `std::process` match is `std::process::id()` inside a Unix permission-test temp-directory name.

Plugin scope is narrow:

- dialog is invoked only by `choose_ics_file`, with user mediation and no caller-supplied path;
- autostart accepts only a boolean and manages the fixed Deskmate LaunchAgent;
- the single-instance callback ignores the second process's arguments and working directory and only shows the fixed settings window (`src-tauri/src/lib.rs:451`).

The config model still contains future `open-url`/`open-application` variants, but compilation returns no wire config for them and save/apply rejects the card as unimplemented (`companion/crates/app-core/src/config.rs:1656`). Consequently no current device event can launch either target.

Rust providers do make intended outbound HTTP(S) requests for user-configured calendar/weather/JSON/RSS sources (`companion/crates/providers/src/http.rs:54`). URLs must be HTTP(S), include a host, and contain no URL userinfo (`providers/src/http.rs:69`); bodies are capped and errors redact transport detail. They are fetched as data and are never opened in a browser or external application. F6 records the local-network reachability policy as a reasoned acceptance.

## Logs

The audit searched `eprintln!`, `println!`, and `log::` in the Tauri shell and `app-core`. `app-core` emits no diagnostic lines. The shell writes only to process stderr; the serial machine stream writes exclusively protocol frames returned by `encode_message` (`companion/crates/device/src/session.rs:449`). No stdout/stderr logger is connected to `SerialTransport`, so diagnostic logs cannot enter the machine-protocol byte stream.

Personal provider values live in `AppSnapshot.card_data`, but exit logging counts provider states and prints numeric device/runtime counters only; it never formats `card_data`, config, provider error messages, or a whole snapshot (`companion/apps/deskmate/src-tauri/src/lib.rs:189`). Calendar summaries and feed values therefore do not reach logs.

The review found two configuration-derived log paths: exit metrics formatted the active card ID and startup formatted `PersistenceState::ValidationFailed`, whose issue messages can include playlist names/IDs or an invalid timezone. F3 fixed both and also converted other shell errors, including untrusted device diagnostics, to closed category labels (`src-tauri/src/commands.rs:127`, `src-tauri/src/lib.rs:201`, `src-tauri/src/lib.rs:355`, `src-tauri/src/events.rs:31`). The full post-fix grep contains only static text, closed labels, aggregate counts, and numeric diagnostics—no secrets, tokens, personal data, provider bodies, config contents, or untrusted strings.

## Config persistence

The path is `~/Library/Application Support/io.deskmate.companion/config.json`, derived from Tauri's application-data directory (`companion/apps/deskmate/src-tauri/src/lib.rs:347`). The observed pre-review modes were:

```text
~/Library/Application Support                         drwx------
~/Library/Application Support/io.deskmate.companion  drwxr-xr-x
.../config.json                                      -rw-r--r--
```

Evidence: `.superpowers/sdd/2026-08-15-deskmate-v1-packaging-hardening/config-perms.txt:3` and `:5`. The `0700` ancestor made the observed configuration effectively accessible only to the owning user, so there is no evidence it was exposed to another local account. The app-owned modes were nevertheless unnecessarily permissive and depended on that ancestor remaining private.

F4 now attempts to create/repair the app directory as `0700` before load (`src-tauri/src/lib.rs:399`) and attempts to repair an existing config to `0600` through its open descriptor before reading (`companion/crates/app-core/src/store.rs:1183`). Both repairs are best-effort: a filesystem that permits reading but rejects `chmod`, or rejects eager directory preparation, cannot discard a valid config or abort startup. Every successful repair still applies the restrictive mode, and every atomic replacement is created as `0600` without preserving a legacy permissive mode (`app-core/src/store.rs:1249`). This protects provider URLs (which may contain query tokens), local calendar paths, labels, playlists, and preferences independently of ancestor modes whenever the filesystem permits enforcement. On non-Unix platforms the existing filesystem ACL model remains unchanged; V1 is macOS-only.

## Release DMG contents

The controller verified the CI disk image checksum and mounted it. At the top level it contained `Deskmate.app`, the expected `Applications -> /Applications` install symlink, and `.VolumeIcon.icns`, which is normal DMG presentation metadata (`.superpowers/sdd/2026-08-15-deskmate-v1-packaging-hardening/dmg-inspection.txt:20`).

`Deskmate.app/Contents` contained only:

- `Info.plist`;
- `MacOS/deskmate-app`;
- `Resources/icon.icns`.

Evidence: `dmg-inspection.txt:27-45`. No source, fixtures, lockfiles, development tools, or other unexpected resources shipped.

`codesign` reported an arm64 bundle with `Signature=adhoc`, no team identifier, an unbound Info.plist, and no sealed resources (`dmg-inspection.txt:46-54`). F5 accepts the absence of Developer ID signing, notarization, and resource sealing for this owner-only dogfood release, exactly as approved by the packaging design. The artifact provides no publisher identity or tamper/authenticity assurance and must not be represented as a public distribution build.

## Findings

| # | Severity | Finding | Disposition |
|---|---|---|---|
| F1 | Low | `core:event:default` granted unused frontend `emit` and `emit_to` IPC permissions. Evidence: pre-review capability plus generated ACL expansion; frontend only listens (`src/lib/tauri.ts:139`). | **Fixed in working tree:** `capabilities/main.json:6` now grants only `allow-listen` and `allow-unlisten`. The Tauri app tests compile and validate the capability. Pending controller commit. |
| F2 | Low | CSP lacked explicit restrictions for directives that do not fall back to `default-src`, leaving form destinations, base URLs, and framing less constrained than intended (`tauri.conf.json:27`). | **Fixed in working tree:** added `object-src 'none'`, `base-uri 'none'`, `form-action 'none'`, and `frame-ancestors 'none'`. The only form is React-handled with `preventDefault()`. Pending controller commit. |
| F3 | Low | Stderr could contain config-derived identifiers/validation values, and other dynamic errors could carry untrusted device diagnostics. Evidence: pre-fix exit `active_screen_id` and `PersistenceState` debug formatting at the code now replaced by `src-tauri/src/lib.rs:201` and `:355`. | **Fixed in working tree:** exit/persistence/error logs now use static state/category labels and aggregate numeric metrics; event-dispatch logs are static. `diagnostic_labels_do_not_include_runtime_or_config_messages` covers representative calendar/config/device strings. Pending controller commit. |
| F4 | Low | App-owned config directory/file modes were `0755`/`0644`; effective access was bounded by the `0700` Application Support ancestor but defense depended on that ancestor. Evidence: `config-perms.txt:3-5`. | **Fixed in working tree:** successful app-directory preparation enforces `0700`; successful existing-file repair and every newly replaced config enforce `0600`. The defense-in-depth repairs are best-effort so inability to change a mode cannot discard a readable valid config or abort startup. Unix tests cover new-save, upgrade repair, injected repair failure, directory modes, and nonfatal directory-preparation failure. Pending controller commit. |
| F5 | Informational | CI artifact is ad-hoc/linker signed, with no Developer ID, notarization, bound Info.plist, or sealed resources (`dmg-inspection.txt:46`). | **Accepted:** V1 is explicitly owner-only dogfood; the approved packaging design chooses ad-hoc signing and no Apple Developer account. Revisit before any external/public distribution. |
| F6 | Informational | User-configured Rust HTTP(S) providers may reach loopback/private-network endpoints; there is no private-address denylist (`providers/src/http.rs:54`). | **Accepted:** reaching a user's local services is intended for the local configurable feed product, configuration is controlled by the local packaged UI, URL userinfo is rejected, responses/time/redirects/parsers are bounded, and fetched URLs are never launched. Revisit if configuration becomes remotely supplied or multi-user/server hosted. |

All findings are fixed or reasoned; none remain open.
