#!/usr/bin/env python3
"""Record isolated copies of stories and make a local visual-review artifact.

Requires Python 3.11+, ffmpeg, and a built matinee. Visual differences are
informational; failed stories or artifact generation return a nonzero status.
"""

import argparse
import html
import hashlib
import json
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tomllib


ROOT = Path(__file__).resolve().parents[1]


def setting(text, section, name, value):
    pattern = rf"(?ms)(^\[{re.escape(section)}\][^\n]*\n)(.*?)(?=^\[|\Z)"
    match = re.search(pattern, text)
    line = f"{name} = {value}\n"
    if not match:
        return text + f"\n[{section}]\n{line}"
    body, count = re.subn(rf"(?m)^\s*{re.escape(name)}\s*=.*\n?", line, match[2])
    if not count:
        body = line + body
    return text[:match.start()] + match[1] + body + text[match.end():]


def ffmpeg(*args):
    subprocess.run(
        ["ffmpeg", "-hide_banner", "-loglevel", "error", "-nostdin", "-y",
         "-threads", "1", "-filter_threads", "1", "-filter_complex_threads", "1",
         *map(str, args)], check=True, timeout=120,
    )


def stills(gif, directory):
    directory.mkdir(parents=True, exist_ok=True)
    ffmpeg("-ignore_loop", "1", "-i", gif,
           "-vf", "fps=fps=1/4:start_time=0:round=near:eof_action=pass,scale=800:-1", "-frames:v", "24",
           "-fps_mode", "vfr", directory / "%03d.png")
    return sorted(directory.glob("*.png"))


def image(path, out, width=400):
    relative = path.relative_to(out).as_posix()
    return f'<a href="{html.escape(relative)}"><img src="{html.escape(relative)}" width="{width}"></a>'


def review(binary, source, out, fixed):
    data = tomllib.loads(source.read_text())
    key = source.parent.name + "-" + source.stem
    dest = out / "stories" / key
    # Fixtures remain beside the copied storyboard. Recordings never replace
    # the checked-in baseline GIFs.
    shutil.copytree(source.parent, dest, dirs_exist_ok=True,
                    ignore=shutil.ignore_patterns("*.gif"))
    config = setting(source.read_text(), "demo", "output", '"clip"')
    if fixed:
        config = setting(config, "camera", "mode", '"fixed"')
    story = dest / ("fixed.toml" if fixed else "review.toml")
    story.write_text(config)
    variant = key + ("-fixed" if fixed else "")
    current = out / "current" / variant
    current.mkdir(parents=True, exist_ok=True)
    # Remove only our previous output files, so a failed rerecord cannot look
    # like a successful run with a stale GIF.
    for theme in data["demo"].get("themes", ["light", "dark"]):
        (dest / f"clip-{theme}.gif").unlink(missing_ok=True)
        (current / f"clip-{theme}.gif").unlink(missing_ok=True)
    log = out / f"{variant}.log"
    with log.open("w") as stream:
        result = subprocess.run([str(binary), "record", "--artifacts",
                                 str(out / "assertions" / variant), str(story)], cwd=ROOT,
                                stdout=stream, stderr=subprocess.STDOUT)
    print(log.read_text(), end="", flush=True)
    rows = [f"<h2>{html.escape(variant)}</h2>",
            f'<p>Story status: {result.returncode}. <a href="{log.name}">Run log</a></p>']
    label = source.relative_to(ROOT) if source.is_relative_to(ROOT) else source
    summary = {"story": str(label), "fixed": fixed,
               "exit_code": result.returncode, "gifs": []}
    snapshots = sorted((out / "assertions" / variant).rglob("*.png"))
    summary["session_stills"] = [str(p.relative_to(out)) for p in snapshots]
    if snapshots:
        rows.append("<details><summary>Actual assertion and failure screens</summary>")
        for snapshot in snapshots:
            rows.append(f"<p>{html.escape(snapshot.parent.name + '/' + snapshot.name)}</p>" + image(snapshot, out))
        rows.append("</details>")
    for theme in data["demo"].get("themes", ["light", "dark"]):
        recorded = dest / f"clip-{theme}.gif"
        if not recorded.exists():
            continue
        gif = current / recorded.name
        shutil.copy2(recorded, gif)
        frames = stills(gif, current / theme)
        summary["gifs"].append({"theme": theme, "bytes": gif.stat().st_size})
        rows.append(f"<h3>{theme}: {gif.stat().st_size / 1024:.0f} KiB</h3>")
        rows.append(image(gif, out, 800))
        rows.append("<details><summary>View at 400 pixels</summary>" + image(gif, out) + "</details>")
        baseline = source.parent / f'{data["demo"]["output"]}-{theme}.gif'
        if baseline.exists() and not fixed:
            base_dir = out / "baseline" / key
            base_dir.mkdir(parents=True, exist_ok=True)
            base_gif = base_dir / gif.name
            shutil.copy2(baseline, base_gif)
            before = stills(base_gif, base_dir / theme)
            rows.append("<p>Sampled baseline / current / absolute pixel difference. "
                        "Timing and live labels can differ; review the full GIFs too.</p>")
            rows.append("<table><tr><th>Baseline</th><th>Current</th><th>Difference</th></tr>")
            for index, (old, new) in enumerate(zip(before, frames), 1):
                diff = current / theme / f"diff-{index:03}.png"
                ffmpeg("-i", old, "-i", new, "-filter_complex",
                       "[0:v][1:v]scale2ref[a][b];[a][b]blend=all_mode=difference",
                       "-frames:v", "1", diff)
                rows.append("<tr>" + "".join(f"<td>{image(p, out)}</td>" for p in (old, new, diff)) + "</tr>")
            rows.append("</table>")
            if len(before) != len(frames):
                rows.append(f"<p>Sample counts differ: {len(before)} baseline, {len(frames)} current.</p>")
        elif fixed:
            rows.append("<p>Fixed-camera comparison: use the matching automatic-camera GIF and size log.</p>")
        else:
            rows.append("<p>No checked-in baseline for this theme.</p>")
    return result.returncode == 0, rows, summary


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=ROOT / "target/release/matinee")
    parser.add_argument("--out", type=Path, default=ROOT / "target/review")
    parser.add_argument("--story", type=Path, action="append", help="repeat to choose stories")
    parser.add_argument("--fixed", action="store_true", help="also record fixed-camera variants")
    args = parser.parse_args()
    binary, out = args.binary.resolve(), args.out.resolve()
    if not binary.is_file() or not shutil.which("ffmpeg"):
        parser.error("build matinee and install ffmpeg before generating the review")
    sources = [p.resolve() for p in (args.story or [
        ROOT / f"examples/{name}/demo.toml" for name in ("terminal", "calculator", "mixed")
    ])]
    keys = [p.parent.name + "-" + p.stem for p in sources]
    if len(set(keys)) != len(keys):
        parser.error("story folder and filename combinations must be unique")
    if out.exists() and any(out.iterdir()):
        parser.error("choose an empty --out directory for a fresh review")
    out.mkdir(parents=True, exist_ok=True)
    # A build may replace the original executable while a recording is in
    # progress. Matinee uses its executable path when creating each private
    # session, so every variant must use one immutable copy.
    pinned = out / "tools" / "matinee"
    pinned.parent.mkdir()
    shutil.copy2(binary, pinned)
    with pinned.open("rb") as stream:
        digest = hashlib.file_digest(stream, "sha256").hexdigest()
    (out / "run.json").write_text(json.dumps({"matinee_sha256": digest}, indent=2) + "\n")
    binary = pinned
    rows = ["<!doctype html><meta charset='utf-8'><title>Matinee visual review</title>",
            "<style>body{font-family:system-ui;margin:2rem}img{max-width:100%;height:auto}"
            "td{vertical-align:top}table{width:100%}</style>",
            "<h1>Matinee visual review</h1><p>Story correctness is the hard gate. "
            "Visual differences and GIF size need human review.</p>"]
    summaries, passed = [], True
    for source in sources:
        for fixed in ([False, True] if args.fixed else [False]):
            try:
                ok, content, summary = review(binary, source, out, fixed)
                rows.extend(content)
                summaries.append(summary)
                passed = passed and ok
            except (OSError, ValueError, subprocess.SubprocessError) as error:
                passed = False
                message = f"{source} ({'fixed' if fixed else 'auto'}): {error}"
                rows.append(f"<p>Artifact error: {html.escape(message)}</p>")
                summaries.append({"story": str(source), "fixed": fixed, "error": str(error)})
                print(message, file=sys.stderr)
    (out / "index.html").write_text("\n".join(rows))
    (out / "summary.json").write_text(json.dumps(summaries, indent=2) + "\n")
    print(f"Visual review: {out / 'index.html'}")
    return 0 if passed else 1


if __name__ == "__main__":
    sys.exit(main())
