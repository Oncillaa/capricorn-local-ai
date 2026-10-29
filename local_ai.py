"""Ответы на вопросы через локальную нейросеть (LM Studio) — без ключа и интернета."""
import datetime
import difflib
import json
import logging
import os
import random
import re
import time
import urllib.request
import urllib.error

from persona import CHARACTER, SPEECH_RULES, user_context

log = logging.getLogger("capri")
URL = "http://127.0.0.1:1234"
SEARCH_MARK = "ПОИСК"
NO_THINKING = "Сразу дай готовый ответ. Без рассуждений, без анализа вопроса."

# Добавили /no_think в конец системного промпта, чтобы модель не тратила время на раздумья
SYSTEM = f"""{CHARACTER}

{SPEECH_RULES}
Отвечай на любые вопросы о науке, истории, технике, языке, о себе, советы и просьбы рассказать что-то.
ВАЖНО: Ты голосовой ассистент. Твои ответы должны быть максимально краткими. Отвечай максимум 2-3 предложениями. Если это математика, сразу пиши ответ.
Только если вопрос про текущие события (погода, новости, курсы), ответь ровно одним словом: {SEARCH_MARK}
/no_think"""

FRESH = re.compile(r"погод|новост|курс (доллар|евро|рубл|валют|биткоин)|сколько стоит|цена|цены|"
                   r"счет матча|кто выиграл|расписани|пробк|сегодня в мире")

FOREIGN = re.compile(r"[　-鿿가-힯＀-￯]+")
THIRD_PERSON = re.compile(r"(одн(а|ой|у|у) прекрасн(ая|ой|ую) (я|мне|меня|мной))", re.IGNORECASE)

def drop_third_person(answer):
    sentences = re.split(r"(?<=[.!?…])\s+", answer)
    clean = [s for s in sentences if not THIRD_PERSON.search(s)]
    if clean:
        return " ".join(clean)
    return THIRD_PERSON.sub("я", answer)

CHATTER_TOPICS = ["пожалуйся, что тебе скучно", "пошути про то, как тесно жить в компьютере", "скажи что-нибудь загадочное"]
SELF_TALK = re.compile(r"\bя\s*[—–-]?\s*капри\b|меня зовут капри|\bкапри (здесь|тут|на связи)|^капри\b", re.IGNORECASE)
ASKED_SELF = re.compile(r"кто ты|как тебя зовут|тво[её] имя|\bии\b|бот|программ|нейросет|о себе|про себя|ты кто", re.IGNORECASE)

def naturalize(answer, question, recent):
    parts = [p for p in re.split(r"(?<=[.!?…])\s+", answer) if p.strip()]
    if not ASKED_SELF.search(question.lower()):
        parts = [p for p in parts if not SELF_TALK.search(p)] or parts
    text = " ".join(parts)
    text = re.sub(r"(^|[.!?…]\s+)(\w)", lambda m: m.group(1) + m.group(2).upper(), text)
    return text.strip() or answer

def _key(sentence):
    return re.sub(r"[^\w ]", "", sentence.lower().replace("ё", "е")).strip()

def drop_repeats(answer, history):
    previous = [_key(s) for m in history if m["role"] == "assistant" for s in re.split(r"(?<=[.!?…])\s+", m["content"])]
    previous = [p for p in previous if len(p) >= 8]
    kept = []
    for s in re.split(r"(?<=[.!?…])\s+", answer):
        k = _key(s)
        if len(k) >= 8 and any(k == p or difflib.SequenceMatcher(None, k, p).ratio() > 0.8 for p in previous):
            continue
        kept.append(s)
    return " ".join(kept) or answer

REASONING = re.compile(r"^\W*(okay|ok,|alright|let me|the user|first,|хорошо,? (пользователь|мне|нужно|давай)|пользователь (спрашива|хочет|проси))", re.IGNORECASE)

def strip_reasoning(text):
    text = re.sub(r"<think>.*?</think>", "", text, flags=re.S)
    if "</think>" in text:
        text = text.rsplit("</think>", 1)[1]
    return re.sub(r"<think>.*", "", text, flags=re.S).strip()

def _post(path, payload, timeout):
    req = urllib.request.Request(URL + path, data=json.dumps(payload).encode("utf-8"),
                                 headers={"Content-Type": "application/json", "Authorization": "Bearer lm-studio"})
    try:
        with urllib.request.urlopen(req, timeout=timeout) as r:
            return json.loads(r.read().decode("utf-8"))
    except urllib.error.HTTPError as e:
        error_body = e.read().decode('utf-8')
        raise Exception(f"HTTP {e.code}: {error_body}")
    except Exception as e:
        raise Exception(f"Ошибка сети: {e}")

PREFERRED = ["qwen3", "qwen2.5", "gemma3", "llama3"]
def has_model(names, model):
    return any(n == model or model in n or n.startswith(model) for n in names)
def pick_model(names):
    chat = [n for n in names if not re.search(r"embed|vision|coder|ocr", n)]
    for p in PREFERRED:
        for n in chat:
            if p in n: return n
    return (chat or [None])[0]

class LocalAI:
    def __init__(self, cfg):
        self.model = cfg.get("local_model", "qwen3:8b")
        self.history = []
        self.on_model_change = None

    @staticmethod
    def installed():
        try:
            with urllib.request.urlopen(URL + "/v1/models", timeout=1.5) as r:
                data = json.loads(r.read().decode("utf-8"))
                return [m["id"] for m in data.get("data", [])]
        except Exception as e:
            log.error("Не могу получить список моделей из LM Studio: %s", e)
            return None

    def start_server(self):
        names = self.installed()
        if names is None:
            log.error("LM Studio не запущен или недоступен на %s", URL)
            return
        
        log.info("LM Studio доступен. Модели: %s", names)
        
        if not has_model(names, self.model):
            log.warning("В LM Studio нет модели '%s'. Ищу замену...", self.model)
            other = pick_model(names)
            if other:
                log.info("Автоматически выбрана модель: %s", other)
                self.model = other
                if self.on_model_change:
                    self.on_model_change(other)
            else:
                log.error("Не удалось найти подходящую модель в LM Studio.")
                return
                
        try:
            _post("/v1/chat/completions", {"model": self.model, "messages": [{"role": "user", "content": "ping"}], "max_tokens": 1}, 30)
            log.info("Модель %s успешно загружена в память", self.model)
        except Exception as e:
            log.error("Не удалось прогреть модель: %s", e)

    # УВЕЛИЧИЛИ ЛИМИТ ДО 1024
    def _chat(self, messages, max_tokens=1024):
        try:
            resp = _post("/v1/chat/completions", {
                "model": self.model,
                "messages": messages,
                "stream": False,
                "temperature": 0.6,
                "max_tokens": max_tokens,
            }, 120)
        except Exception as e:
            log.error("Сбой при запросе к LM Studio: %s", e)
            raise e
        
        # ИЗМЕНЕНО: Берем текст из поля "content" (готовый ответ), а не из reasoning_content
        try:
            message = resp["choices"][0]["message"]
            raw_text = message.get("content", "")
            reasoning_text = message.get("reasoning_content", "")
        except (KeyError, IndexError):
            log.error("Странный ответ от LM Studio (нет choices): %s", resp)
            return ""

        # Если content пустой, значит модель потратила все токены на мысли
        if not raw_text and reasoning_text:
            log.warning("Модель думала, но не успела ответить (finish_reason: %s). Увеличьте max_tokens.", resp["choices"][0].get("finish_reason"))
            return ""

        text = strip_reasoning(raw_text)

        if REASONING.search(text):
            log.info("Модель рассуждает вслух вместо ответа. Пробую переспросить.")
            if messages[-1].get("content", "").startswith(NO_THINKING):
                return ""
            return self._chat(messages + [{"role": "system", "content": NO_THINKING}], max_tokens)
        return text

    def ask(self, question):
        if FRESH.search(question.lower()):
            return None
        now = datetime.datetime.now().strftime("%d.%m.%Y %H:%M")
        self.history.append({"role": "user", "content": question})
        self.history = self.history[-6:]
        messages = [{"role": "system", "content": SYSTEM + "\n" + user_context() + f"\nСейчас {now}."}] + self.history
        
        try:
            answer = self._chat(messages)
            if FOREIGN.search(answer):
                answer = self._chat(messages + [{"role": "system", "content": "Отвечай строго по-русски, без иероглифов."}])
            answer = re.sub(r"\s+", " ", FOREIGN.sub("", answer)).strip()
        except Exception as e:
            log.error("ОШИБКА ИИ: %s", e)
            self.history.pop()
            return None
            
        recent = [m["content"] for m in self.history if m["role"] == "assistant"]
        answer = naturalize(drop_repeats(drop_third_person(answer), self.history), question, recent)
        answer = re.sub(r"\s*\([^)]*\)", "", answer)
        answer = re.sub(r"[\U0001F000-\U0001FAFF☀-➿️]", "", answer)
        answer = re.sub(r"\s+", " ", answer).strip()
        answer = answer[:1].upper() + answer[1:]
        
        if not answer or SEARCH_MARK in answer.upper()[:20]:
            log.warning("Ответ отброшен как пустой или содержащий метку поиска.")
            self.history.pop()
            return None
        self.history.append({"role": "assistant", "content": answer})
        return answer

    def chatter(self):
        try:
            text = self._chat([{"role": "system", "content": SYSTEM + "\n" + user_context()}, {"role": "user", "content": f"Скажи короткую фразу. Тема: {random.choice(CHATTER_TOPICS)}."}], max_tokens=200)
        except Exception:
            return None
        text = drop_third_person(re.sub(r"\s+", " ", FOREIGN.sub("", text)).strip().strip('"«»'))
        return text if 5 < len(text) < 160 else None

    def summarize_report(self, report):
        try:
            return self._chat([{"role": "system", "content": SYSTEM}, {"role": "user", "content": "Отчёт ПК:\n" + report[:12000]}], max_tokens=500) or None
        except Exception:
            return None