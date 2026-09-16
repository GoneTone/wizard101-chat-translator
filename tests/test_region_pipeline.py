"""本機 OCR → 文字翻譯的決策；translator 與 OCR 都用替身。"""
import pytest

from src.region.ocr import OcrUnavailable
from src.region.pipeline import RegionPipeline, RegionResult
from src.translation.translator import TranslatorConfigError, TranslatorOffline

_PNG = b"png"
_RECT = (10, 20, 300, 100)


class FakeTranslator:
    def __init__(self, text=None, text_raises=None):
        self._text, self._text_raises = text, text_raises
        self.text_calls = []

    def translate_region_text(self, text, cancel=None):
        self.text_calls.append(text)
        self.last_cancel = cancel
        if self._text_raises:
            raise self._text_raises
        return self._text


def test_ocr_text_is_translated_and_source_is_kept():
    translator = FakeTranslator(text="譯文")
    pipeline = RegionPipeline(translator, recognize=lambda png: "Hello")
    assert pipeline.run(_PNG, _RECT) == RegionResult("譯文", "Hello")
    assert translator.text_calls == ["Hello"]


def test_ocr_without_text_returns_an_empty_result_without_translating():
    translator = FakeTranslator()
    pipeline = RegionPipeline(translator, recognize=lambda png: "")
    assert pipeline.run(_PNG, _RECT) == RegionResult("", "")
    assert translator.text_calls == []


def test_ocr_unavailable_propagates():
    translator = FakeTranslator()

    def unavailable(png):
        raise OcrUnavailable("no language pack")

    with pytest.raises(OcrUnavailable):
        RegionPipeline(translator, recognize=unavailable).run(_PNG, _RECT)


def test_translator_config_error_propagates():
    translator = FakeTranslator(text_raises=TranslatorConfigError("bad key", status=401))
    pipeline = RegionPipeline(translator, recognize=lambda png: "Hello")
    with pytest.raises(TranslatorConfigError):
        pipeline.run(_PNG, _RECT)


def test_translator_offline_error_propagates():
    translator = FakeTranslator(text_raises=TranslatorOffline("down", status=503))
    pipeline = RegionPipeline(translator, recognize=lambda png: "Hello")
    with pytest.raises(TranslatorOffline):
        pipeline.run(_PNG, _RECT)


def test_the_cancel_handle_is_passed_to_the_translator():
    translator = FakeTranslator(text="譯文")
    handle = object()
    RegionPipeline(translator, recognize=lambda png: "Hello").run(_PNG, _RECT, cancel=handle)
    assert translator.last_cancel is handle
