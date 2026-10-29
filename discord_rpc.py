import time
import threading
import logging
from pypresence import Presence
from pynput import keyboard, mouse

log = logging.getLogger("capri")

CLIENT_ID = "1565095712646234272" # ВАШ ID

class DiscordRPC:
    def __init__(self):
        self.rpc = None
        self.keys_count = 0
        self.clicks_count = 0
        self.start_time = int(time.time())
        self.level = 4
        self.running = True
        self.connected = False
        
        threading.Thread(target=self._init_rpc, daemon=True).start()
        threading.Thread(target=self._count_keys, daemon=True).start()
        threading.Thread(target=self._count_clicks, daemon=True).start()
        threading.Thread(target=self._update_loop, daemon=True).start()

    def _init_rpc(self):
        while self.running:
            if not self.connected:
                try:
                    self.rpc = Presence(CLIENT_ID)
                    self.rpc.connect()
                    self.connected = True
                    log.info("Discord RPC подключен")
                except Exception as e:
                    log.error("Ошибка Discord RPC (ожидание Discord): %s", e)
                    time.sleep(15) # Ждем 15 секунд и пробуем снова
            time.sleep(5)

    def _count_keys(self):
        def on_press(key):
            self.keys_count += 1
        with keyboard.Listener(on_press=on_press) as listener:
            listener.join()

    def _count_clicks(self):
        def on_click(x, y, button, pressed):
            if pressed:
                self.clicks_count += 1
        with mouse.Listener(on_click=on_click) as listener:
            listener.join()

    def _update_loop(self):
        while self.running:
            if self.connected and self.rpc:
                try:
                    self.rpc.update(
                        state="Пьёт синтетический кофе",
                        details=f"Ур. {self.level} · Сегодня: {self.keys_count} клавиши, {self.clicks_count} кликов",
                        start=self.start_time,
                        large_image="capri_logo", 
                        large_text="Capricorn"
                    )
                except Exception as e:
                    log.error("Сбой обновления Discord RPC: %s", e)
                    self.connected = False
                    try:
                        self.rpc.close()
                    except: pass
            time.sleep(15) 

    def stop(self):
        self.running = False
        if self.connected and self.rpc:
            try:
                self.rpc.clear()
                self.rpc.close()
            except: pass