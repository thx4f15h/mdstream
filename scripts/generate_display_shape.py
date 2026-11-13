#!/usr/bin/env python3
"""Generate locked display transducers offline; invoke inside the workspace devShell."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
TEMPLATES = ROOT / 'scripts/display_shape'
OUTPUT = ROOT / 'crates/mdtui-tui/src/render/shape_tables.rs'
PROVENANCE = ROOT / 'data/unicode/display-shape-provenance.json'

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    metadata = json.loads(subprocess.check_output(['cargo', 'metadata', '--format-version', '1']))
    packages = {p['name']: p for p in metadata['packages']}
    sources = {}
    for name, version in [('unicode-segmentation', '1.13.3'), ('unicode-width', '0.2.3')]:
        p = packages[name]
        assert p['version'] == version
        sources[name] = Path(p['manifest_path']).parent / 'src'
    inputs = {}
    for name, src in sources.items():
        for p in sorted(src.rglob('*.rs')):
            inputs[f'{name}/{p.relative_to(src)}'] = hashlib.sha256(p.read_bytes()).hexdigest()
    if PROVENANCE.exists():
        assert json.loads(PROVENANCE.read_text())['crate_source_sha256'] == inputs
    with tempfile.TemporaryDirectory(prefix='mdtui-shape-generator-') as tmp:
        tmp = Path(tmp)
        substitutions = {'@SEG_SRC@':str(sources['unicode-segmentation']), '@WIDTH_SRC@':str(sources['unicode-width']), '@GCB@':str(TEMPLATES/'gcb.rs'), '@WIDTH_STATES@':(TEMPLATES/'width-states.json').read_text().strip()}
        for name in ['main.rs', 'width_info.rs']:
            source = (TEMPLATES/(name+'.in')).read_text()
            for key, value in substitutions.items(): source = source.replace(key, value)
            (tmp/name).write_text(source)
        subprocess.run(['rustc','--edition=2021','-O',str(tmp/'main.rs'),'-o',str(tmp/'generate')],check=True)
        data = subprocess.check_output([str(tmp/'generate')])
    # Use the same checked-in rustfmt policy as production.
    formatted = subprocess.check_output(['rustfmt','--edition','2024','--config-path',str(ROOT/'.rustfmt.toml')],input=data)
    if args.check:
        assert OUTPUT.read_bytes() == formatted
    else:
        OUTPUT.write_bytes(formatted)
        PROVENANCE.write_text(json.dumps({'gcb_crate':'unicode-segmentation 1.13.3','gcb_unicode':'17.0.0','width_crate':'unicode-width 0.2.3','width_unicode':'18.0.0','width_mode':'default-features=false; non-CJK','width_reachable_states_source':'018 accepted exhaustive closure, 146792448 scalar/state transitions; fixed list in scripts/display_shape/width-states.json','gcb_oracle':'https://www.unicode.org/Public/17.0.0/ucd/auxiliary/GraphemeBreakTest.txt','gcb_oracle_sha256':'e2d134d2c52919bace503ebb6a551c1855fe1a1faec18478c78fff254a1793ec','crate_source_sha256':inputs,'generated_sha256':hashlib.sha256(formatted).hexdigest()},indent=2)+'\n')
if __name__ == '__main__': main()
