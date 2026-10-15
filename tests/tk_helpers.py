"""Tk 測試共用的事件迴圈輔助。"""
import _tkinter


def settle_events(win, rounds=3):
    """送完視窗事件（頂層改尺寸後的 <Configure> 是事件，idle 佇列送不到）與 idle，不跑計時器。"""
    for _ in range(rounds):
        win.update_idletasks()
        while win.tk.dooneevent(_tkinter.WINDOW_EVENTS | _tkinter.DONT_WAIT):
            pass
    win.update_idletasks()
