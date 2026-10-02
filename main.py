import tkinter as tk
from tkinter import filedialog, messagebox
from tkinterdnd2 import TkinterDnD, DND_FILES
from PIL import Image, ImageTk
import os
from pathlib import Path
from image_processor import ImageProcessor

class ImageProcessorApp:
    def __init__(self, root):
        self.root = root
        self.root.title("Image Processor")
        self.font_path = str(Path(__file__).resolve().parent / "assets" / "fonts" / "IBMPlexMono-Regular.ttf")
        self.cell_size = 20
        self.font_size = 20

        self.filepath = None
        self.image_processor = ImageProcessor(self.font_path)

        self.create_widgets()

    def create_widgets(self):
        self.frame = tk.Frame(self.root)
        self.root.resizable(False, False)
        self.frame.pack(padx=10, pady=10)
        self.image_frame = tk.Frame(self.frame)
        self.image_frame.grid(row=0, column=4, rowspan=6, padx=10)
        # Frame for drag and drop
        self.drop_frame = tk.Frame(self.frame, width=400, height=200, bg='lightgrey')
        self.drop_frame.grid(row=0, columnspan=3, pady=10)
        self.drop_frame.pack_propagate(False)
        self.drop_frame.drop_target_register(DND_FILES)
        self.drop_frame.dnd_bind('<<Drop>>', self.drop)

        self.label = tk.Label(self.drop_frame, text="Drag and drop an image here", bg='lightgrey')
        self.label.pack(expand=True)

        # Button for file dialog
        self.open_button = tk.Button(self.frame, text="Open Image", command=self.open_file_dialog)
        self.open_button.grid(row=1, columnspan=3, pady=10)

        # Sliders and entries for cell size
        self.cell_size_label = tk.Label(self.frame, text="Cell Size:")
        self.cell_size_label.grid(row=2, column=0, pady=5)
        self.cell_size_slider = tk.Scale(self.frame, from_=5, to_=30, orient=tk.HORIZONTAL, command=self.update_cell_size)
        self.cell_size_slider.set(self.cell_size)
        self.cell_size_slider.grid(row=2, column=1, pady=5)
        self.cell_size_entry = tk.Entry(self.frame)
        self.cell_size_entry.insert(0, str(self.cell_size))
        self.cell_size_entry.grid(row=2, column=2, pady=5)
        self.cell_size_entry.bind("<Return>", self.update_cell_size_entry)

        # Slider and entry for font size
        self.font_size_label = tk.Label(self.frame, text="Font Size:")
        self.font_size_label.grid(row=3, column=0, pady=5)
        self.font_size_slider = tk.Scale(self.frame, from_=8, to_=30, orient=tk.HORIZONTAL, command=self.update_font_size)
        self.font_size_slider.set(self.font_size)
        self.font_size_slider.grid(row=3, column=1, pady=5)
        self.font_size_entry = tk.Entry(self.frame)
        self.font_size_entry.insert(0, str(self.font_size))
        self.font_size_entry.grid(row=3, column=2, pady=5)
        self.font_size_entry.bind("<Return>", self.update_font_size_entry)

        # Labels to display the original and processed images
        self.original_image_label = tk.Label(self.image_frame)
        self.original_image_label.pack(side=tk.LEFT, padx=10)

        self.processed_image_label = tk.Label(self.image_frame)
        self.processed_image_label.pack(side=tk.LEFT, padx=10)

        # Button to save the processed image
        self.save_button = tk.Button(self.frame, text="Save Image", command=self.save_image)
        self.save_button.grid(row=5, columnspan=3, pady=10)
         # Button to reset the image selection
        self.reset_button = tk.Button(self.frame, text="Reset", command=self.reset_image)
        self.reset_button.grid(row=6, columnspan=3, pady=10)

    def open_file_dialog(self):
        self.filepath = filedialog.askopenfilename(filetypes=[("Image files", "*.jpg;*.jpeg;*.png;*.bmp")])
        if self.filepath:
            self.label.config(text=self.filepath)
            self.load_image()

    def drop(self, event):
        self.filepath = event.data.strip("{}")
        if os.path.isfile(self.filepath):
            self.label.config(text=self.filepath)
            self.load_image()
        else:
            messagebox.showerror("Invalid file", "The dropped file is invalid or does not exist.")

    def update_cell_size(self, event=None):
        cell_size = self.cell_size_slider.get()
        self.cell_size_entry.delete(0, tk.END)
        self.cell_size_entry.insert(0, str(cell_size))
        self.cell_size = cell_size
        self.update_image()

    def update_cell_size_entry(self, event):
        try:
            cell_size = int(self.cell_size_entry.get())
            if 8 <= cell_size <= 30:
                self.cell_size_slider.set(cell_size)
                self.cell_size = cell_size
                self.update_image()
        except ValueError:
            messagebox.showerror("Invalid input", "Cell size must be an integer between 8 and 30.")

    def update_font_size(self, event=None):
        font_size = self.font_size_slider.get()
        self.font_size_entry.delete(0, tk.END)
        self.font_size_entry.insert(0, str(font_size))
        self.font_size = font_size
        self.update_image()

    def update_font_size_entry(self, event):
        try:
            font_size = int(self.font_size_entry.get())
            if 8 <= font_size <= 30:
                self.font_size_slider.set(font_size)
                self.font_size = font_size
                self.update_image()
        except ValueError:
            messagebox.showerror("Invalid input", "Font size must be an integer between 8 and 30.")

    def load_image(self):
        try:
            self.original_img = Image.open(self.filepath)
            self.display_image(self.original_img, self.original_image_label)
            self.update_image()
        except Exception as e:
            messagebox.showerror("Error opening image", str(e))

    def update_image(self, event=None):
        if self.filepath:
            processed_img = self.image_processor.process_image(self.filepath, self.cell_size, self.font_size)
            self.processed_img_fullsize = processed_img.copy()  # сохраняем полноразмерное изображение
            self.display_image(processed_img, self.processed_image_label)
            self.processed_img = processed_img
            
    def display_image(self, img, label):
        img.thumbnail((256, 256), Image.LANCZOS)
        tk_img = ImageTk.PhotoImage(img)
        label.config(image=tk_img)
        label.image = tk_img

    def reset_image(self):
        self.filepath = None
        self.label.config(text="Drag and drop an image here")
        self.original_image_label.config(image='')
        self.processed_image_label.config(image='')
        self.cell_size_slider.set(20)
        self.font_size_slider.set(20)
        self.cell_size_entry.delete(0, tk.END)
        self.cell_size_entry.insert(0, '20')
        self.font_size_entry.delete(0, tk.END)
        self.font_size_entry.insert(0, '20')

    def save_image(self):
        if hasattr(self, 'processed_img_fullsize'):
            save_path = filedialog.asksaveasfilename(defaultextension=".png", filetypes=[("PNG files", "*.png")])
            if save_path:
                self.processed_img_fullsize.save(save_path)
        else:
            messagebox.showerror("No image", "There is no image to save.")

if __name__ == "__main__":
    root = TkinterDnD.Tk()
    app = ImageProcessorApp(root)
    root.mainloop()
