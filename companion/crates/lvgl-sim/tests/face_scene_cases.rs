use std::collections::BTreeSet;

use lvgl_sim::{SimTemplate, Simulator, cases};

#[test]
fn promoted_face_scene_inventory_is_complete_and_byte_exact() {
    let template_cases = cases::golden_cases();
    let scene_cases = cases::face_scene_cases();
    assert_eq!(template_cases.len(), 58);
    assert_eq!(scene_cases.len(), template_cases.len());

    let mut templates = BTreeSet::new();
    let mut sim = Simulator::new().expect("simulator");
    for ((template_name, template), (scene_name, scene)) in
        template_cases.into_iter().zip(scene_cases)
    {
        assert_eq!(scene_name, template_name);
        templates.insert(match template.template {
            SimTemplate::DigitalClock => "digital-clock",
            SimTemplate::ProgressRing => "progress-ring",
            SimTemplate::RowList => "row-list",
            SimTemplate::AnalogClock => "analog-clock",
            SimTemplate::BigNumberLabel => "big-number-label",
            SimTemplate::IconBadgeText => "icon-badge-text",
        });

        // The running mid-countdown row is intentionally hardware-excluded:
        // the C oracle advances from LVGL's private fake-tick anchor while a
        // scene consumes the supplied snapshot. All deterministic rows must
        // remain byte-identical here, or the promoted matrix is not a valid
        // device-vs-simulator expectation.
        if template_name.starts_with("progress-ring--running-mid-countdown--") {
            continue;
        }
        let template_pixels = sim.render(&template).expect(&template_name);
        let scene_pixels = sim.render_scene(&scene).expect(&scene_name);
        assert_eq!(scene_pixels, template_pixels, "{template_name}");
    }

    assert_eq!(templates.len(), 6);
}
