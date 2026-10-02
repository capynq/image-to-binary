"""Freeze processor behavior and exercise Tk callbacks without opening a window."""

import ast
import os
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import Mock, patch

from PIL import Image, ImageFont

from image_processor import ImageProcessor


ROOT = Path(__file__).resolve().parents[1]
FONT = ROOT / "assets" / "fonts" / "IBMPlexMono-Regular.ttf"


def headless_app_class():
    """Compile the original class with fake UI boundaries, not a second implementation."""
    tree = ast.parse((ROOT / "main.py").read_text(encoding="utf-8"))
    definition = next(node for node in tree.body if isinstance(node, ast.ClassDef))
    module = ast.Module(body=[definition], type_ignores=[])
    namespace = {
        "__file__": str(ROOT / "main.py"),
        "Path": Path,
        "os": os,
        "tk": SimpleNamespace(END="end"),
        "messagebox": Mock(),
        "filedialog": Mock(),
        "ImageProcessor": ImageProcessor,
    }
    exec(compile(module, str(ROOT / "main.py"), "exec"), namespace)
    return namespace["ImageProcessorApp"]


class ProcessorBaselineTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.path = Path(self.directory.name) / "source with spaces.png"
        self.processor = ImageProcessor(str(FONT))

    def source(self, size=(40, 20), mode="L", color=0):
        image = Image.new(mode, size, color)
        image.save(self.path)
        return image

    def glyphs(self):
        with patch("image_processor.ImageDraw.Draw") as draw:
            result = self.processor.process_image(self.path, 20, 16)
        return result, draw.return_value.text.call_args_list

    def test_threshold_boundary_maps_dark_to_one(self):
        image = self.source((80, 20))
        image.paste(127, (20, 0, 40, 20))
        image.paste(128, (40, 0, 60, 20))
        image.paste(255, (60, 0, 80, 20))
        image.save(self.path)
        result, calls = self.glyphs()
        self.assertEqual(result.size, (80, 20))
        self.assertEqual([call.kwargs["text"] for call in calls], ["1", "1", "0", "0"])
        self.assertEqual([call.args[0] for call in calls], [(0, 0), (20, 0), (40, 0), (60, 0)])
        self.assertTrue(all(call.kwargs["fill"] == (0, 255, 0, 255) for call in calls))

    def test_classic_sampling_is_nearest_neighbor_not_average(self):
        image = self.source((20, 20), color=255)
        image.putpixel((10, 10), 0)
        image.save(self.path)
        _, calls = self.glyphs()
        self.assertEqual(calls[0].kwargs["text"], "1")

    def test_partial_cells_use_floor_dimensions(self):
        self.source((59, 39))
        result, calls = self.glyphs()
        self.assertEqual(result.size, (40, 20))
        self.assertEqual(len(calls), 2)

    def test_rgba_alpha_is_ignored_by_classic_grayscale(self):
        self.source(mode="RGBA", color=(0, 0, 0, 0))
        _, calls = self.glyphs()
        self.assertEqual([call.kwargs["text"] for call in calls], ["1", "1"])

    def test_classic_does_not_apply_exif_orientation(self):
        image = Image.new("RGB", (40, 20), (127, 127, 127))
        orientation = image.getexif()
        orientation[274] = 6
        self.path = self.path.with_suffix(".jpg")
        image.save(self.path, exif=orientation)
        result, _ = self.glyphs()
        self.assertEqual(result.size, (40, 20))

    def test_render_is_repeatable_green_on_opaque_black(self):
        self.source()
        first = self.processor.process_image(self.path, 20, 16)
        second = self.processor.process_image(self.path, 20, 16)
        self.assertEqual(first.tobytes(), second.tobytes())
        self.assertEqual(first.mode, "RGBA")
        pixels = first.tobytes()
        self.assertTrue(any(pixels[1::4]))
        self.assertFalse(any(pixels[0::4]))
        self.assertFalse(any(pixels[2::4]))
        self.assertTrue(all(alpha == 255 for alpha in pixels[3::4]))

    def test_bundled_font_is_loadable(self):
        self.assertGreater(ImageFont.truetype(str(FONT), 16).getlength("01"), 0)

    def test_corrupt_source_raises_instead_of_exporting(self):
        self.path.write_bytes(b"not an image")
        with self.assertRaises(OSError):
            self.processor.process_image(self.path, 20, 16)

    @unittest.expectedFailure
    def test_known_defect_tiny_image_should_have_nonzero_output(self):
        self.source((1, 1))
        result = self.processor.process_image(self.path, 20, 16)
        self.assertGreater(result.width, 0)


class CallbackBaselineTests(unittest.TestCase):
    def setUp(self):
        self.app_class = headless_app_class()
        self.app = self.app_class.__new__(self.app_class)
        for name in (
            "label", "original_image_label", "processed_image_label",
            "cell_size_slider", "font_size_slider", "cell_size_entry", "font_size_entry",
        ):
            setattr(self.app, name, Mock())
        self.app.update_image = Mock()

    def test_font_path_is_relative_to_module_not_working_directory(self):
        with patch.object(self.app_class, "create_widgets"):
            app = self.app_class(Mock())
        self.assertEqual(Path(app.font_path), FONT)

    def test_slider_callback_processes_synchronously(self):
        self.app.cell_size_slider.get.return_value = 5
        self.app.update_cell_size()
        self.assertEqual(self.app.cell_size, 5)
        self.app.update_image.assert_called_once_with()

    def test_known_entry_slider_range_mismatch_is_preserved(self):
        self.app.cell_size = 20
        self.app.cell_size_entry.get.return_value = "5"
        self.app.update_cell_size_entry(None)
        self.assertEqual(self.app.cell_size, 20)
        self.app.update_image.assert_not_called()

    @unittest.expectedFailure
    def test_known_defect_reset_should_clear_exportable_image(self):
        self.app.processed_img_fullsize = Image.new("RGBA", (20, 20))
        self.app.reset_image()
        self.assertIsNone(getattr(self.app, "processed_img_fullsize", None))

    @unittest.expectedFailure
    def test_known_defect_multiple_drop_should_not_become_one_path(self):
        self.app.filepath = None
        self.app.load_image = Mock()
        self.app.drop(SimpleNamespace(data="{one image.png} {two image.png}"))
        self.assertIsNone(self.app.filepath)


if __name__ == "__main__":
    unittest.main()
