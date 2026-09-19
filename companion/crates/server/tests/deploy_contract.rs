#[test]
fn example_firmware_version_matches_the_repository_version() {
    let example = include_str!("../deploy/deskmate-server.env.example");
    let firmware_version = include_str!("../../../../firmware/version.txt")
        .strip_suffix("\r\n")
        .or_else(|| include_str!("../../../../firmware/version.txt").strip_suffix('\n'))
        .expect("firmware/version.txt must end with a line ending");
    let assignments: Vec<_> = example
        .lines()
        .map(str::trim_start)
        .filter(|line| !line.starts_with('#'))
        .filter_map(|line| line.strip_prefix("DESKMATE_FIRMWARE_VERSION="))
        .collect();

    assert_eq!(
        assignments.len(),
        1,
        "the example must contain exactly one uncommented DESKMATE_FIRMWARE_VERSION assignment"
    );
    assert_eq!(assignments[0], firmware_version);
}
