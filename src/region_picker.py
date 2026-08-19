"""全螢幕半透明遮罩,拖曳框選聊天框區域。Esc 取消。"""
import tkinter as tk


def _to_region(x0: int, y0: int, x1: int, y1: int) -> dict:
    return {
        "left": min(x0, x1),
        "top": min(y0, y1),
        "width": max(abs(x1 - x0), 1),
        "height": max(abs(y1 - y0), 1),
    }


def pick_region() -> dict | None:
    root = tk.Tk()
    root.attributes("-fullscreen", True)
    root.attributes("-alpha", 0.35)
    root.attributes("-topmost", True)
    root.configure(bg="black", cursor="crosshair")

    canvas = tk.Canvas(root, bg="black", highlightthickness=0)
    canvas.pack(fill="both", expand=True)
    canvas.create_text(
        root.winfo_screenwidth() // 2, 60,
        text="拖曳框選遊戲聊天框範圍(Esc 取消)",
        fill="white", font=("Microsoft JhengHei", 16),
    )

    state: dict = {"start": None, "rect": None, "result": None}

    def on_press(e):
        state["start"] = (e.x_root, e.y_root)
        state["rect"] = canvas.create_rectangle(e.x, e.y, e.x, e.y, outline="#00d0ff", width=2)

    def on_drag(e):
        if state["rect"] is not None:
            x0, y0 = state["start"]
            canvas.coords(state["rect"], x0, y0, e.x_root, e.y_root)

    def on_release(e):
        if state["start"] is not None:
            x0, y0 = state["start"]
            state["result"] = _to_region(x0, y0, e.x_root, e.y_root)
        root.destroy()

    def on_escape(_e):
        state["result"] = None
        root.destroy()

    canvas.bind("<ButtonPress-1>", on_press)
    canvas.bind("<B1-Motion>", on_drag)
    canvas.bind("<ButtonRelease-1>", on_release)
    root.bind("<Escape>", on_escape)
    root.mainloop()
    return state["result"]
