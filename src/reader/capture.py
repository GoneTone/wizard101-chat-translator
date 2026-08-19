"""以 mss 截取聊天框區域,轉為 PIL Image 並放大以利 OCR。"""
import mss
from PIL import Image


def grab_region(region: dict, scale: int = 2) -> Image.Image:
    with mss.mss() as sct:
        shot = sct.grab({
            "left": region["left"], "top": region["top"],
            "width": region["width"], "height": region["height"],
        })
    img = Image.frombytes("RGB", shot.size, shot.bgra, "raw", "BGRX")
    if scale != 1:
        img = img.resize((img.width * scale, img.height * scale), Image.LANCZOS)
    return img
