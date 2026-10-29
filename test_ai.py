import urllib.request
import json

URL = "http://127.0.0.1:1234/v1/chat/completions"
MODEL = "ВАШЕ_НАЗВАНИЕ_МОДЕЛИ_ИЗ_LM_STUDIO" # Вставьте сюда название из config.json

print(f"Стучусь в LM Studio на {URL}...")
try:
    req = urllib.request.Request(URL, data=json.dumps({
        "model": MODEL,
        "messages": [{"role": "user", "content": "Скажи 'Привет'"}],
        "max_tokens": 50
    }).encode("utf-8"), headers={"Content-Type": "application/json"})
    
    with urllib.request.urlopen(req, timeout=10) as r:
        print("УСПЕХ! Сервер ответил:")
        print(r.read().decode("utf-8"))
except Exception as e:
    print(f"ОШИБКА: {e}")