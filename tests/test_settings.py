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
