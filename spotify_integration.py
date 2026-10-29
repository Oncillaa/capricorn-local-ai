import asyncio
import logging

log = logging.getLogger("capri")


class SpotifyWatcher:
    def __init__(self):
        self.current_track = None
        self.is_playing = False

    async def _get_media_info(self):
        try:
            from winrt.windows.media.control import (
                GlobalSystemMediaTransportControlsSessionManager,
            )

            sessions = (
                await GlobalSystemMediaTransportControlsSessionManager.request_async()
            )
            session = sessions.get_current_session()
            if session:
                info = await session.try_get_media_properties_async()
                playback_info = session.get_playback_info()

                # 4 означает PLAYING в Windows API
                is_playing = playback_info.playback_status == 4
                if is_playing:
                    return f"{info.title} — {info.artist}"
            return None
        except Exception:
            return None

    def check(self):
        """Синхронный вызов для фонового потока"""
        try:
            loop = asyncio.new_event_loop()
            asyncio.set_event_loop(loop)
            track = loop.run_until_complete(self._get_media_info())
            loop.close()

            if track != self.current_track:
                self.current_track = track
                self.is_playing = bool(track)
                return track
        except Exception:
            pass
        return None
