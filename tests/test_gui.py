"""The GUI opens a window, paints it, and exits cleanly on SIGTERM. Needs a display."""

import os
import signal
import subprocess
import time
from pathlib import Path

import pytest
from Xlib import X, display, error

REPOSITORY_ROOT = Path(__file__).resolve().parent.parent
DEFAULT_GUI_BINARY = REPOSITORY_ROOT / "gui/src-tauri/target/release/dcpdoctor-gui"

# tauri.conf.json names the window "DcpDoctor - DCP Validation Suite". The other
# window tauri maps carries the binary name and is a few pixels wide.
WINDOW_TITLE = "DcpDoctor"
MIN_WINDOW_SIDE = 200

WINDOW_TIMEOUT_SECONDS = 60
PAINT_TIMEOUT_SECONDS = 60
EXIT_TIMEOUT_SECONDS = 15
POLL_SECONDS = 0.5


def read_command(*command):
    return subprocess.run(command, capture_output=True, text=True)


def window_geometry(screen, window_id):
    """The window's geometry, or None when it is gone or holds no pixels."""
    try:
        geometry = screen.create_resource_object("window", window_id).get_geometry()
    except error.XError:
        return None
    # an InputOnly window, such as the compositor's, has depth 0
    return geometry if geometry.depth else None


def app_window(screen):
    """The app's own readable window id, or None while none is up yet."""
    found = read_command("xdotool", "search", "--onlyvisible", "--name", WINDOW_TITLE)
    for found_id in found.stdout.split():
        window_id = int(found_id)
        geometry = window_geometry(screen, window_id)
        if geometry and min(geometry.width, geometry.height) >= MIN_WINDOW_SIDE:
            return window_id
    return None


def distinct_pixels(screen, window_id):
    """How many distinct pixel values the window's contents hold."""
    geometry = window_geometry(screen, window_id)
    if geometry is None:
        return 0
    window = screen.create_resource_object("window", window_id)
    # only the part on screen can be read, a window larger than the screen
    # makes the whole read fail
    root_window = screen.screen().root
    root = root_window.get_geometry()
    on_root = window.translate_coords(root_window, 0, 0)
    width = min(geometry.width, root.width - on_root.x)
    height = min(geometry.height, root.height - on_root.y)
    try:
        grabbed = window.get_image(0, 0, width, height, X.ZPixmap, 0xFFFFFFFF)
    except error.XError:
        return 0
    pixels = bytes(grabbed.data)
    stride = len(pixels) // (width * height)
    return len({pixels[at : at + stride] for at in range(0, len(pixels), stride)})


def gui_process_environment(config_home):
    environment = dict(os.environ)
    # the app runs under XWayland on a wayland desktop, and under Xvfb in CI
    environment["GDK_BACKEND"] = "x11"
    # a fresh config, so the developer's saved window size and preferences
    # stay out of the run
    environment["XDG_CONFIG_HOME"] = str(config_home)
    return environment


def fail_with_output(message, process, log):
    process.kill()
    output = log.read_text(errors="replace") if log.exists() else ""
    pytest.fail(f"{message}\n--- gui output ---\n{output[-4000:]}")


def test_gui_opens_paints_and_exits(tmp_path):
    binary = Path(os.environ.get("DCPDOCTOR_GUI", DEFAULT_GUI_BINARY))
    assert binary.is_file(), f"GUI binary not found at {binary}"
    assert os.environ.get("DISPLAY"), "no DISPLAY: run under xvfb-run or a desktop"

    screen = display.Display()
    log = tmp_path / "gui.log"
    with log.open("wb") as sink:
        process = subprocess.Popen(
            [str(binary)],
            stdout=sink,
            stderr=subprocess.STDOUT,
            env=gui_process_environment(tmp_path / "config"),
        )

    try:
        deadline = time.monotonic() + WINDOW_TIMEOUT_SECONDS
        window_id = None
        while time.monotonic() < deadline and window_id is None:
            if process.poll() is not None:
                fail_with_output(
                    f"the GUI exited with {process.returncode} before a window appeared",
                    process,
                    log,
                )
            time.sleep(POLL_SECONDS)
            window_id = app_window(screen)
        if window_id is None:
            fail_with_output(
                f"no window titled {WINDOW_TITLE!r} in {WINDOW_TIMEOUT_SECONDS}s",
                process,
                log,
            )

        # the window is created on the backgroundColor tauri.conf.json sets, so
        # one distinct pixel means the webview has not drawn the page yet
        deadline = time.monotonic() + PAINT_TIMEOUT_SECONDS
        colours = 0
        while time.monotonic() < deadline and colours < 2:
            if process.poll() is not None:
                fail_with_output(
                    f"the GUI exited with {process.returncode} before it painted",
                    process,
                    log,
                )
            time.sleep(POLL_SECONDS)
            colours = distinct_pixels(screen, window_id)
        if colours < 2:
            fail_with_output(
                f"the window stayed one colour for {PAINT_TIMEOUT_SECONDS}s",
                process,
                log,
            )

        process.send_signal(signal.SIGTERM)
        try:
            process.wait(timeout=EXIT_TIMEOUT_SECONDS)
        except subprocess.TimeoutExpired:
            fail_with_output(
                f"the GUI ignored SIGTERM for {EXIT_TIMEOUT_SECONDS}s", process, log
            )
        assert process.returncode in (
            0,
            -signal.SIGTERM,
        ), f"unclean exit {process.returncode}"
    finally:
        screen.close()
        if process.poll() is None:
            process.kill()
            process.wait(timeout=EXIT_TIMEOUT_SECONDS)
