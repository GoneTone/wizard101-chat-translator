"""ServiceForm：單筆服務編輯表單的行為。"""
import pytest

from src.services import EFFORT_LOW, new_service
from src.ui.service_form import ServiceForm


@pytest.fixture
def blank(root):
    return ServiceForm(root, new_service("openai", []))


def test_new_form_starts_from_the_providers_defaults(blank):
    values = blank.values()
    assert values["provider"] == "openai"
    assert values["name"] == "ChatGPT"
    assert values["model"] == ""


def test_editing_keeps_the_id(root):
    service = new_service("claude", [])
    service.update(name="我的 Claude", model="claude-x", api_key="sk-ant")
    form = ServiceForm(root, service)
    assert form.values()["id"] == service["id"]
    assert form.values()["name"] == "我的 Claude"
    assert form.values()["model"] == "claude-x"


def test_values_only_carry_the_fields_that_provider_has(root):
    form = ServiceForm(root, new_service("claude", []))
    assert set(form.values()) == {"id", "name", "provider", "model", "api_key", "effort"}


def test_api_values_drop_the_bookkeeping_fields(blank):
    assert "id" not in blank.api_values() and "name" not in blank.api_values()


def test_switching_provider_rewrites_an_untouched_name(blank):
    blank.set_provider("claude")
    assert blank.values()["name"] == "Claude"


def test_switching_provider_keeps_a_name_the_user_typed(blank):
    blank.set_name("戰鬥用")
    blank.set_provider("claude")
    assert blank.values()["name"] == "戰鬥用"


def test_a_blank_name_falls_back_to_the_provider_short_name(blank):
    blank.set_name("   ")
    assert blank.values()["name"] == "ChatGPT"


def test_switching_provider_does_not_carry_values_across(root):
    service = new_service("openai", [])
    service.update(model="gpt-x", api_key="sk-1")
    form = ServiceForm(root, service)
    form.set_provider("claude")
    assert form.values()["model"] == ""
    assert form.values()["api_key"] == ""


def test_claude_keeps_its_effort_choice(root):
    service = new_service("claude", [])
    service["effort"] = EFFORT_LOW
    form = ServiceForm(root, service)
    assert form.values()["effort"] == EFFORT_LOW
