#[test]
    fn passenger_animation_is_saved_and_restored() {
        assert_eq!(settings_from_text(None)["passenger_animation"], "enhanced");
        // (an older file's `original` was the old default: taken as the new one)
        assert_eq!(settings_from_text(Some("version=2\npassenger_animation=original\n"))["passenger_animation"], "enhanced");
        assert_eq!(settings_from_text(Some("version=3\npassenger_animation=original\n"))["passenger_animation"], "original");
        let mut settings = settings_from_text(None);
        settings["passenger_animation"] = json!("enhanced");
        let saved = settings_to_text(&settings, Some("passenger_animation=original\n"));
        assert_eq!(saved.lines().filter(|s| s.starts_with("passenger_animation=")).count(), 1);
        assert_eq!(settings_from_text(Some(&saved))["passenger_animation"], "enhanced");
        assert_eq!(settings_from_text(Some("version=3\npassenger_animation=unknown\n"))["passenger_animation"], "original");
    }
