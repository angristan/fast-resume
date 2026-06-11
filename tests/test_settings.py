"""Tests for persistent user settings."""

from fast_resume.settings import load_settings, save_settings


class TestSettings:
    """Tests for load_settings / save_settings."""

    def test_save_then_load_round_trips(self, temp_dir):
        path = temp_dir / "settings.json"
        save_settings({"preview_height": 21}, path=path)

        loaded = load_settings(path=path)
        assert loaded["preview_height"] == 21

    def test_load_missing_file_returns_defaults(self, temp_dir):
        path = temp_dir / "does-not-exist.json"
        loaded = load_settings(path=path)
        assert loaded["preview_height"] == 12

    def test_load_corrupt_file_returns_defaults(self, temp_dir):
        path = temp_dir / "settings.json"
        path.write_text("{ not valid json")
        loaded = load_settings(path=path)
        assert loaded["preview_height"] == 12

    def test_load_merges_unknown_keys_with_defaults(self, temp_dir):
        path = temp_dir / "settings.json"
        save_settings({"something_else": "x"}, path=path)
        loaded = load_settings(path=path)
        # Missing known keys fall back to defaults
        assert loaded["preview_height"] == 12

    def test_save_creates_parent_directory(self, temp_dir):
        path = temp_dir / "nested" / "dir" / "settings.json"
        save_settings({"preview_height": 9}, path=path)
        assert load_settings(path=path)["preview_height"] == 9


class TestLegacyMigration:
    """Tests for migrating settings.json from the old cache-dir location."""

    def _patch_paths(self, monkeypatch, temp_dir):
        import fast_resume.settings as settings_mod

        new = temp_dir / "config" / "settings.json"
        legacy = temp_dir / "cache" / "settings.json"
        monkeypatch.setattr(settings_mod, "SETTINGS_FILE", new)
        monkeypatch.setattr(settings_mod, "LEGACY_SETTINGS_FILE", legacy)
        return new, legacy

    def test_legacy_file_is_moved_to_config_dir(self, temp_dir, monkeypatch):
        from fast_resume.settings import _migrate_legacy_settings

        new, legacy = self._patch_paths(monkeypatch, temp_dir)
        legacy.parent.mkdir(parents=True)
        legacy.write_text('{"preview_height": 25}')

        _migrate_legacy_settings()

        assert not legacy.exists()
        assert load_settings(path=new)["preview_height"] == 25

    def test_existing_config_file_is_not_overwritten(self, temp_dir, monkeypatch):
        from fast_resume.settings import _migrate_legacy_settings

        new, legacy = self._patch_paths(monkeypatch, temp_dir)
        legacy.parent.mkdir(parents=True)
        legacy.write_text('{"preview_height": 25}')
        new.parent.mkdir(parents=True)
        new.write_text('{"preview_height": 30}')

        _migrate_legacy_settings()

        assert load_settings(path=new)["preview_height"] == 30
        assert legacy.exists()

    def test_no_legacy_file_is_a_noop(self, temp_dir, monkeypatch):
        from fast_resume.settings import _migrate_legacy_settings

        new, legacy = self._patch_paths(monkeypatch, temp_dir)
        _migrate_legacy_settings()
        assert not new.exists()
