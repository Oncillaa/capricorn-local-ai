import json
import os

SAVE_FILE = "save_data.json"

DEFAULT_DATA = {
    "money": 328,
    "level": 4,
    "xp": 98,
    "xp_to_next": 180,
    "mood_joy": 92,
    "mood_tired": 46,
    "equipped_skin": "purple",
    "owned_skins": ["purple"],
}


class SaveManager:
    @staticmethod
    def load():
        if not os.path.exists(SAVE_FILE):
            SaveManager.save(DEFAULT_DATA)
            return DEFAULT_DATA
        try:
            with open(SAVE_FILE, "r", encoding="utf-8") as f:
                return json.load(f)
        except Exception:
            return DEFAULT_DATA

    @staticmethod
    def save(data):
        with open(SAVE_FILE, "w", encoding="utf-8") as f:
            json.dump(data, f, ensure_ascii=False, indent=4)
