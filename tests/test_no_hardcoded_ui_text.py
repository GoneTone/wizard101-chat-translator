"""防迴歸：UI 模組內不得再出現中日韓文字的字串字面量。

忘了走 i18n 就會直接失敗。刻意不掃 translation/prompts.py（LLM 提示詞）與 mem_reader.py（英文 log）。
"""
import ast
from pathlib import Path

import pytest

SCAN_TARGETS = [
    "src/main.py",
    "src/config.py",
    "src/ui/overlay.py",
    "src/ui/message_list.py",
    "src/ui/input_box.py",
    "src/ui/fields.py",
    "src/ui/form.py",
    "src/ui/model_field.py",
    "src/services.py",
    "src/ui/provider_picker.py",
    "src/ui/service_form.py",
    "src/ui/service_list.py",
    "src/ui/settings.py",
    "src/ui/wizard.py",
    "src/ui/fonts.py",
    "src/ui/responsive.py",
    "src/ui/region_card.py",
    "src/ui/monitors.py",
    "src/ui/region_select.py",
    "src/ui/region_flow.py",
    "src/ui/tooltip.py",
    "src/translation/pool.py",
    "src/translation/translator.py",
]

# 例外：語言選單與翻譯目標語言清單一律顯示 endonym，任何介面語言下都不翻譯。
# 全形空白（U+3000）是卡片標題列拿來對齊 ● 的等寬留白，不是可譯文字。
ALLOWED = {
    "繁體中文（台灣）", "简体中文（中国）", "日本語", "한국어",
    "　",
}


def _has_cjk(text: str) -> bool:
    return any("぀" <= c <= "鿿" or "가" <= c <= "힯"
               or "　" <= c <= "〿" or "＀" <= c <= "￯"
               for c in text)


def _docstring_nodes(tree: ast.AST) -> set[int]:
    """模組／函式／類別的 docstring 節點 id：docstring 用中文是規範，不該被擋。"""
    ids = set()
    for node in ast.walk(tree):
        if (isinstance(node, (ast.Module, ast.FunctionDef, ast.AsyncFunctionDef,
                              ast.ClassDef))
                and ast.get_docstring(node, clean=False) is not None
                and node.body and isinstance(node.body[0], ast.Expr)):
            ids.add(id(node.body[0].value))
    return ids


@pytest.mark.parametrize("relative_path", SCAN_TARGETS)
def test_no_hardcoded_cjk_string_literals(relative_path):
    path = Path(__file__).resolve().parent.parent / relative_path
    tree = ast.parse(path.read_text(encoding="utf-8"))
    skip = _docstring_nodes(tree)
    offenders = [
        (node.lineno, node.value)
        for node in ast.walk(tree)
        if isinstance(node, ast.Constant) and isinstance(node.value, str)
        and id(node) not in skip and _has_cjk(node.value)
        and node.value not in ALLOWED
    ]
    assert offenders == [], (
        f"{relative_path} 仍有硬編的介面文字，請改走 src.i18n.t()：{offenders}")
