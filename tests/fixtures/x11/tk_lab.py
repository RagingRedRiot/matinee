"""An X11-only window: Tk has no Wayland backend. Save writes the entry's
text to ~/typed, so the story can check what reached the app."""

import os
import tkinter as tk

root = tk.Tk()
root.title("Tk lab")
entry = tk.Entry(root, font=("TkDefaultFont", 14))
entry.place(x=20, y=20, width=300, height=32)
status = tk.Label(root, text="")
status.place(x=140, y=70, height=32)


def save():
    with open(os.path.expanduser("~/typed"), "w") as f:
        f.write(entry.get())
    status.config(text="saved")


tk.Button(root, text="Save", command=save).place(x=20, y=70, width=100, height=32)
root.mainloop()
