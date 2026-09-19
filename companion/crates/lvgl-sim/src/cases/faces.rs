use protocol::Scene;

use super::orientations;
use crate::scene::{SceneRenderRequest, SceneTimer};

/// 2025-08-13 11:35:00 UTC — the "widest-looking" of the tabular pair.
pub const TABULAR_1135: i64 = 1_755_084_900;
/// 2025-08-13 00:00:00 UTC — the "narrowest-looking" of the tabular pair.
pub const TABULAR_0000: i64 = 1_755_043_200;

/// The shipping-face rows as device-pushable scenes.
///
/// This is both the golden table (`tests/golden.rs` pins the landscape rows
/// under `tests/golden/`) and the hardware-facing half of the framebuffer
/// matrix that `framebuffer_diff` drives.
///
/// The field matrix is expressed directly as scene-builder inputs. Row names
/// are stable because `framebuffer_diff` exclusions are keyed by them; a
/// rename would silently make an exclusion branch unreachable.
#[allow(clippy::too_many_lines)] // one explicit row per shipped face case
pub fn face_scene_cases() -> Vec<(String, SceneRenderRequest)> {
    use app_core::scene_build::{
        AnalogClockCard, BakedFontMetrics, ClockCard, ProgressRingCard, SceneDataState,
        build_analog_clock_scene, build_digital_clock_scene, build_progress_ring_scene,
        with_scene_data_state,
    };

    /// 2025-08-12 12:00:00 UTC.
    const NOW: i64 = 1_755_000_000;
    /// -> 16:00 local.
    const OFFSET: i16 = 240;
    /// The analog brief's pinned instant. `typical` and `no-seconds` share it
    /// on purpose: together they pin that the hour and minute hands do not
    /// depend on `show_seconds`, which is the deterministic form of the
    /// 2026-08-11 no-seconds defect.
    const ANALOG_INSTANT: i64 = 1_755_081_480;
    /// 2025-08-13 00:00:00 UTC.
    const MIDNIGHT_INSTANT: i64 = 1_755_043_200;
    const DURATION_SECONDS: i64 = 1500;

    let metrics = &BakedFontMetrics::SHIPPED;

    let clock = |show_seconds: bool, now_unix_seconds: i64, utc_offset_minutes: i16| {
        let local_seconds = now_unix_seconds + i64::from(utc_offset_minutes) * 60;
        let local_now = chrono::DateTime::from_timestamp(local_seconds, 0)
            .expect("golden clock instant is representable")
            .naive_utc();
        build_digital_clock_scene(
            &ClockCard {
                revision: 1,
                show_seconds,
                local_now,
            },
            metrics,
        )
    };
    let analog = |show_seconds: bool| {
        build_analog_clock_scene(&AnalogClockCard {
            revision: 1,
            show_seconds,
        })
    };
    let ring = |remaining_seconds: i64, running: bool| {
        let scene = build_progress_ring_scene(
            &ProgressRingCard {
                revision: 1,
                label: "Pomodoro",
                duration_seconds: DURATION_SECONDS,
            },
            metrics,
        );
        let to_milliseconds = |seconds: i64| {
            u32::try_from(seconds * 1_000).expect("progress-ring fixture is inside u32")
        };
        (
            scene,
            Some(SceneTimer {
                total_ms: to_milliseconds(DURATION_SECONDS),
                remaining_ms: to_milliseconds(remaining_seconds),
                running,
            }),
        )
    };

    let rows: Vec<(&'static str, Scene, Option<SceneTimer>, i64, i16)> = vec![
        (
            "digital-clock--typical",
            clock(true, NOW, OFFSET),
            None,
            NOW,
            OFFSET,
        ),
        (
            "digital-clock--no-seconds",
            clock(false, NOW, OFFSET),
            None,
            NOW,
            OFFSET,
        ),
        // Tabular-figure pair. Both instants render an HH:MM with no repeated
        // digit shape in common, at the same font and the same box, so their
        // lit column spans must be identical -- proportional figures would
        // shift the second frame. `tests/tabular.rs` asserts that equality in
        // pixels; these goldens pin the frames it asserts over.
        (
            "digital-clock--tabular-1135",
            clock(false, TABULAR_1135, 0),
            None,
            TABULAR_1135,
            0,
        ),
        (
            "digital-clock--tabular-0000",
            clock(false, TABULAR_0000, 0),
            None,
            TABULAR_0000,
            0,
        ),
        (
            "analog-clock--typical",
            analog(true),
            None,
            ANALOG_INSTANT,
            0,
        ),
        (
            "analog-clock--no-seconds",
            analog(false),
            None,
            ANALOG_INSTANT,
            0,
        ),
        (
            "analog-clock--midnight",
            analog(true),
            None,
            MIDNIGHT_INSTANT,
            0,
        ),
    ];

    // A running ring mid-countdown is the only pixel coverage of the running
    // arc-indicator hue: `running` drives the indicator and status-text
    // colours, and at a zero-length arc the indicator is not drawn at all. It
    // is golden-only -- `framebuffer_diff` excludes it by name, because the
    // device keeps ticking a running timer through `scene_view_tick_bindings`
    // while this simulator's fake tick is fixed.
    let ring_rows: Vec<(&'static str, (Scene, Option<SceneTimer>))> = vec![
        ("progress-ring--running-mid-countdown", ring(900, true)),
        // The hardware-comparable half of the row above: the same partial arc
        // with the ring stopped, so both sides render the pinned value.
        ("progress-ring--paused-mid-countdown", ring(900, false)),
        // The only running ring hardware can be compared on, and so the only
        // on-device coverage of the running palette -- here the status text,
        // since a zero-length arc draws no indicator.
        ("progress-ring--running-at-zero", ring(0, true)),
        ("progress-ring--finished", ring(0, false)),
        (
            "progress-ring--never-started",
            ring(DURATION_SECONDS, false),
        ),
    ];

    let mut cases = Vec::new();
    let all = rows.into_iter().chain(
        ring_rows
            .into_iter()
            .map(|(name, (scene, timer))| (name, scene, timer, NOW, 0)),
    );
    for (name, scene, timer, now_unix_seconds, utc_offset_minutes) in all {
        let scene = with_scene_data_state(
            scene,
            SceneDataState {
                stale: false,
                error: None,
            },
            metrics,
        );
        for (orientation_slug, orientation) in orientations() {
            cases.push((
                format!("{name}--{orientation_slug}"),
                SceneRenderRequest {
                    scene: scene.clone(),
                    assets: Vec::new(),
                    utc_offset_minutes,
                    now_unix_seconds,
                    timer,
                    orientation,
                },
            ));
        }
    }
    cases
}
