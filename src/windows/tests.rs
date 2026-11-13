use super::*;

#[test]
#[ignore = "requires DISCIMAGE_TEST_OPTICAL_DRIVE and an idle physical Windows drive"]
fn live_windows_reservation_rejects_other_process_and_releases() {
    let id = std::env::var("DISCIMAGE_TEST_OPTICAL_DRIVE")
        .expect("set DISCIMAGE_TEST_OPTICAL_DRIVE to the selected optical drive letter");
    const CHILD: &str = "DISCIMAGE_TEST_RESERVATION_CHILD";
    if let Ok(mode) = std::env::var(CHILD) {
        let reserved = Imapi::new(&id).and_then(|mut link| {
            link.reserve()?;
            Ok(link)
        });
        if mode == "blocked" {
            assert!(
                reserved.is_err(),
                "another process acquired the reserved drive"
            );
        } else {
            reserved.expect("exclusive access was not released");
        }
        return;
    }
    let child = |mode: &str| {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "windows::tests::live_windows_reservation_rejects_other_process_and_releases",
                "--ignored",
                "--nocapture",
            ])
            .env(CHILD, mode)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "child {mode}: {}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    };
    let owner = ProbeDevice::open(&id).expect("cannot reserve idle test drive");
    child("blocked");
    drop(owner);
    child("available");
}
