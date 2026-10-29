#!/usr/bin/env python3
"""Staging HTML -> markdown via any2md, then check nothing was lost.

    python convert.py                    # data/staging -> data/out

any2md would try to download every <img src> over HTTP and drop the ones it
cannot fetch, so image refs are swapped for placeholders before the call and put
back after. The rest is post-processing of two any2md quirks (see below).
"""
import argparse
import os
import re
import shutil
import subprocess
import sys
from collections import Counter
from html import unescape
from pathlib import Path

ROOT = Path(__file__).parent
STAGING = ROOT / 'data' / 'staging'
OUT = ROOT / 'data' / 'out'
ANY2MD = os.environ.get('ANY2MD_BIN', str(ROOT / 'vendor' / 'any2md' / 'target' / 'release' / 'any2md'))

IMG_TAG = re.compile(r'<img\b[^>]*\bsrc="([^"]+)"[^>]*>', re.I)
PLACEHOLDER = re.compile(r'\[\[IMG:([^\]]+)\]\]')
# A table with no <th> gets an empty header row plus its separator, and with no
# header cells to size them both lines come out as a bare "|  |". Dropping them
# alone would leave the table with no separator at all — which stops being a table
# — so the first real row is promoted to header and a sized separator inserted.
EMPTY_HEAD = re.compile(
    r'^\|(?:\s*\|)+[ \t]*\n\|(?:\s*(?:-{3})?\s*\|)+[ \t]*\n(\|.*\n)', re.M)


def promote_header(m):
    row = m.group(1)
    cells = row.strip().strip('|').split('|')
    return row + '|' + '|'.join([' --- '] * len(cells)) + '|\n'


def coverage(src_html, md):
    """Character multiset comparison — did any text get dropped on the way?

    Same idea as the PDF pipeline's gate: word-level comparison is useless because
    conversion legitimately re-splits words, characters are stable.

    Tags are stripped from the HTML side only. Markdown has none, and a literal
    `<-` in one cell plus a `>` pages later would be eaten as one giant "tag" —
    that exact false loss (60% on a clean page) blocked the first full run.
    """
    def count(text):
        return Counter(c for c in re.sub(r'[^0-9a-zà-ɏ]', '', text.lower()))
    src = count(unescape(re.sub(r'<[^>]+>', ' ', src_html)))
    got = count(re.sub(r'!\[[^\]]*\]\([^)]*\)', ' ', md))
    total = sum(src.values()) or 1
    lost = sum((src - got).values())
    return (total - lost) / total, lost, total


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--staging', default=str(STAGING))
    ap.add_argument('--out', default=str(OUT))
    ap.add_argument('--min-coverage', type=float, default=0.99)
    args = ap.parse_args()

    staging, out = Path(args.staging), Path(args.out)
    if not staging.exists():
        print(f'nothing staged in {staging} — run export.py first', file=sys.stderr)
        return 3
    if out.exists():
        shutil.rmtree(out)     # out mirrors staging exactly; stale notes must not
    out.mkdir(parents=True)    # survive here or vault deletions can never happen

    worst, failures, done = 1.0, [], 0
    for html_path in sorted(staging.rglob('*.html')):
        rel = html_path.relative_to(staging)
        dest = out / rel.with_suffix('.md')
        dest.parent.mkdir(parents=True, exist_ok=True)

        html = html_path.read_text()
        refs = []

        def stash(m):
            src = m.group(1)
            if src.startswith('data:'):
                return ''
            refs.append(src)
            return f'<p>[[IMG:{src}]]</p>'

        tmp_html = dest.with_suffix('.tmp.html')
        tmp_html.write_text(IMG_TAG.sub(stash, html))
        tmp_md = dest.with_suffix('.tmp.md')
        proc = subprocess.run([ANY2MD, str(tmp_html), '-o', str(tmp_md)],
                              capture_output=True, text=True)
        if proc.returncode != 0 or not tmp_md.exists():
            print(f'FAIL {rel}: {proc.stderr.strip().splitlines()[-1:] or proc.returncode}')
            failures.append(str(rel))
            tmp_html.unlink(missing_ok=True)
            continue

        md = tmp_md.read_text()
        md = EMPTY_HEAD.sub(promote_header, md)
        md = PLACEHOLDER.sub(lambda m: f'![image]({m.group(1)})', md)
        md = re.sub(r'\n{3,}', '\n\n', md).strip() + '\n'
        dest.write_text(md)
        tmp_html.unlink(missing_ok=True)
        tmp_md.unlink(missing_ok=True)

        ratio, lost, total = coverage(html, md)
        worst = min(worst, ratio)
        flag = 'ok  ' if ratio >= args.min_coverage else 'LOSS'
        if ratio < args.min_coverage:
            failures.append(f'{rel} (coverage {ratio:.1%})')
        print(f'{flag} {ratio:6.1%} lost={lost:5d}/{total:6d} imgs={len(refs)}  {rel}')
        done += 1

    # copy the images the exporter already wrote, unchanged (content-addressed)
    for img in staging.rglob('images/**/*'):
        if img.is_file():
            dst = out / img.relative_to(staging)
            dst.parent.mkdir(parents=True, exist_ok=True)
            dst.write_bytes(img.read_bytes())

    print(f'\n{done} converted, worst coverage {worst:.1%}')
    if failures:
        print('failures:', *failures, sep='\n  ')
        return 1
    return 0


if __name__ == '__main__':
    sys.exit(main())
