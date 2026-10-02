import numpy as np
from PIL import Image, ImageDraw, ImageFont


class ImageProcessor:
    def __init__(self, font_path):
        self.font_path = font_path

    def process_image(self, filepath, cell_size, font_size):
        img = Image.open(filepath)
        img = img.convert("L")  # Преобразование в градации серого
        img = img.resize(
            (img.width // cell_size, img.height // cell_size), Image.NEAREST
        )

        img_array = np.array(img)
        binary_array = (img_array < 128).astype(int)

        new_img = Image.new(
            "RGBA", (img.width * cell_size, img.height * cell_size), (0, 0, 0, 255)
        )
        d = ImageDraw.Draw(new_img)
        font = ImageFont.truetype(self.font_path, font_size)

        for i in range(binary_array.shape[0]):
            for j in range(binary_array.shape[1]):
                text = "1" if binary_array[i, j] == 1 else "0"
                d.text(
                    (j * cell_size, i * cell_size),
                    text=text,
                    font=font,
                    fill=(0, 255, 0, 255),
                )

        return new_img
