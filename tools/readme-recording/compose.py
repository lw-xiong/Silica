"""frames/<clip>/*.png (grim captures of the headless output) -> <out>.gif via ffmpeg,
plus a PNG still for every frame that carries a name."""
import json, os, subprocess, sys
ROOT = os.environ.get("SILICA_REC_WORK", os.path.dirname(os.path.abspath(__file__)))
clip, out = sys.argv[1], sys.argv[2]
width = int(sys.argv[3]) if len(sys.argv) > 3 else 1000
d = os.path.join(ROOT, "frames", clip)
frames = json.load(open(os.path.join(d, "durations.json")))
comp = os.path.join(d, "comp"); os.makedirs(comp, exist_ok=True)
concat = []
for f in frames:
    src = os.path.join(d, f["file"]); dst = os.path.join(comp, f["file"])
    subprocess.run(["magick", src, "-filter", "Lanczos", "-resize", f"{width}x", dst], check=True)
    concat.append(f"file '{dst}'\nduration {f['dur']}")
concat.append(f"file '{dst}'")
lst = os.path.join(d, "concat.txt"); open(lst, "w").write("\n".join(concat) + "\n")
for f in frames:
    if f.get("name"):
        still = os.path.join(os.path.dirname(out), f["name"] + ".png")
        subprocess.run(["magick", os.path.join(d, f["file"]), "-filter", "Lanczos", "-resize", "1600x", "-colors", "256", still], check=True)
filt = "split[a][b];[a]palettegen=max_colors=256:stats_mode=diff[p];[b][p]paletteuse=dither=sierra2_4a:diff_mode=rectangle"
subprocess.run(["ffmpeg", "-y", "-loglevel", "error", "-f", "concat", "-safe", "0", "-i", lst, "-vf", filt, "-loop", "0", out], check=True)
print(out, os.path.getsize(out) // 1024, "KB,", len(frames), "frames")
