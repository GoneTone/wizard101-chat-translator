import tkinter as tk

import pytest

from src.ui.popup import Popup, clamped_position


@pytest.fixture
def popup(root):
    clicks = []
    p = Popup(root, on_click=lambda: clicks.append(1))
    yield p, clicks
    p.destroy()


def test_clamped_position_keeps_a_fitting_popup_where_it_is():
    assert clamped_position(100, 200, 120, 40, 1920, 1080, margin=4) == (100, 200)


def test_clamped_position_pulls_back_from_the_right_edge():
    x, _ = clamped_position(1900, 200, 120, 40, 1920, 1080, margin=4)
    assert x == 1920 - 120 - 4


def test_clamped_position_pulls_back_from_the_bottom_edge():
    _, y = clamped_position(100, 1070, 120, 40, 1920, 1080, margin=4)
    assert y == 1080 - 40 - 4


def test_clamped_position_never_goes_past_the_top_left():
    assert clamped_position(-50, -50, 120, 40, 1920, 1080, margin=4) == (4, 4)


def test_show_makes_it_visible_with_the_given_label(popup, root):
    p, _ = popup
    assert p.visible is False

    p.show(300, 300, "複製")
    root.update()

    assert p.visible is True
    assert p.label_text() == "複製"


def test_label_follows_what_is_passed_each_time(popup, root):
    p, _ = popup
    p.show(300, 300, "複製")
    root.update()
    p.hide()

    p.show(300, 300, "Copy")
    root.update()

    assert p.label_text() == "Copy"


def test_hide_makes_it_invisible(popup, root):
    p, _ = popup
    p.show(300, 300, "複製")
    root.update()

    p.hide()
    root.update()

    assert p.visible is False


def test_clicking_the_row_fires_the_callback_and_closes(popup, root):
    p, clicks = popup
    p.show(300, 300, "複製")
    root.update()

    p._clicked(None)
    root.update()

    assert clicks == [1]
    assert p.visible is False


def test_the_popup_never_takes_keyboard_focus(popup, root):
    # 取走焦點的話疊加視窗的 Ctrl+C 就收不到了；Esc 因此也由 overlay 綁在本體上
    p, _ = popup
    before = root.focus_get()

    p.show(300, 300, "複製")
    root.update()

    assert root.focus_get() is before


def test_hover_swaps_the_row_colours(popup, root):
    p, _ = popup
    p.show(300, 300, "複製")
    root.update()
    resting = p._label.cget("bg")

    p._hover(True)
    hovered = p._label.cget("bg")

    assert hovered != resting
    p._hover(False)
    assert p._label.cget("bg") == resting


def test_hiding_an_already_hidden_popup_is_harmless(popup, root):
    p, _ = popup
    p.hide()
    root.update()
    assert p.visible is False


def test_the_popup_is_a_borderless_topmost_window(popup):
    p, _ = popup
    # 疊加視窗自己就是 overrideredirect + topmost；選單得比它更上層，否則會被蓋掉
    assert isinstance(p._win, tk.Toplevel)
    assert bool(p._win.overrideredirect()) is True
    assert bool(p._win.attributes("-topmost")) is True
