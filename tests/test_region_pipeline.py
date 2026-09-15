"""看圖優先、退回本機 OCR 的決策；translator 與 OCR 都用替身。"""
import pytest

from src.region.ocr import OcrUnavailable
from src.region.pipeline import RegionPipeline, RegionResult
from src.translation.translator import TranslatorConfigError, TranslatorOffline

_PNG = b"png"
_RECT = (10, 20, 300, 100)


class FakeTranslator:
    def __init__(self, image=("", ""), image_raises=None, text=None, text_raises=None):
        self._image, self._image_raises = image, image_raises
        self._text, self._text_raises = text, text_raises
        self.image_calls = 0
        self.text_calls = []

    def translate_region_image(self, png):
        """回傳 (原文, 譯文)，比照 Translator.translate_region_image 的介面。"""
        self.image_calls += 1
        if self._image_raises:
            raise self._image_raises
        return self._image

    def translate_region_text(self, text):
        self.text_calls.append(text)
        if self._text_raises:
            raise self._text_raises
        return self._text


def test_image_path_returns_the_translation_without_touching_ocr():
    translator = FakeTranslator(image=("原文", "譯文"))
    ocr_calls = []
    pipeline = RegionPipeline(translator, recognize=lambda png: ocr_calls.append(png) or "x")
    assert pipeline.run(_PNG, _RECT) == RegionResult("譯文", "image", "原文")
    assert ocr_calls == [] and not pipeline.text_only


def test_image_path_with_empty_original_and_translation_shows_no_text():
    # 畫面上根本沒有文字：抄寫與譯文都留空，卡片走「no text」提示
    translator = FakeTranslator(image=("", ""))
    pipeline = RegionPipeline(translator, recognize=lambda png: "x")
    assert pipeline.run(_PNG, _RECT) == RegionResult("", "image", "")


def test_rejected_image_falls_back_to_ocr_and_marks_text_only_when_text_succeeds():
    translator = FakeTranslator(image_raises=TranslatorConfigError("no images", status=400),
                                text="譯文")
    pipeline = RegionPipeline(translator, recognize=lambda png: "Hello")
    assert pipeline.run(_PNG, _RECT) == RegionResult("譯文", "ocr", "Hello")
    assert translator.text_calls == ["Hello"]
    assert pipeline.text_only
    # 已標記：之後直接走 OCR，不再多送一趟圖片
    pipeline.run(_PNG, _RECT)
    assert translator.image_calls == 1


def test_rejected_image_with_failing_text_does_not_mark_text_only():
    translator = FakeTranslator(image_raises=TranslatorConfigError("bad key", status=401),
                                text_raises=TranslatorConfigError("bad key", status=401))
    pipeline = RegionPipeline(translator, recognize=lambda png: "Hello")
    with pytest.raises(TranslatorConfigError):
        pipeline.run(_PNG, _RECT)
    assert not pipeline.text_only


def test_server_error_on_the_image_falls_back_without_marking_text_only():
    # 實測 OpenAI 對純文字模型收到圖片回 500：這一輪照樣退回 OCR，但 5xx 可能只是暫時故障，不標記
    translator = FakeTranslator(image_raises=TranslatorOffline("server error", status=500),
                                text="譯文")
    pipeline = RegionPipeline(translator, recognize=lambda png: "Hello")
    assert pipeline.run(_PNG, _RECT) == RegionResult("譯文", "ocr", "Hello")
    assert not pipeline.text_only
    pipeline.run(_PNG, _RECT)
    assert translator.image_calls == 2   # 沒標記：下一次仍先試圖片


def test_offline_endpoint_raises_the_text_path_error():
    translator = FakeTranslator(image_raises=TranslatorOffline("down", status=503),
                                text_raises=TranslatorOffline("down", status=503))
    pipeline = RegionPipeline(translator, recognize=lambda png: "Hello")
    with pytest.raises(TranslatorOffline):
        pipeline.run(_PNG, _RECT)
    assert not pipeline.text_only


def test_ocr_without_text_returns_an_empty_result_without_translating():
    translator = FakeTranslator(image_raises=TranslatorConfigError("no images", status=400))
    pipeline = RegionPipeline(translator, recognize=lambda png: "")
    assert pipeline.run(_PNG, _RECT) == RegionResult("", "ocr")
    assert translator.text_calls == [] and not pipeline.text_only


def test_ocr_unavailable_propagates():
    translator = FakeTranslator(image_raises=TranslatorConfigError("no images", status=400))

    def unavailable(png):
        raise OcrUnavailable("no language pack")

    with pytest.raises(OcrUnavailable):
        RegionPipeline(translator, recognize=unavailable).run(_PNG, _RECT)


def test_reset_clears_the_text_only_mark():
    translator = FakeTranslator(image_raises=TranslatorConfigError("no images", status=400),
                                text="譯文")
    pipeline = RegionPipeline(translator, recognize=lambda png: "Hello")
    pipeline.run(_PNG, _RECT)
    assert pipeline.text_only
    pipeline.reset()
    assert not pipeline.text_only
    pipeline.run(_PNG, _RECT)
    assert translator.image_calls == 2
