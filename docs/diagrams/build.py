"""Regenerates the diagrams under docs/public: python3 docs/diagrams/build.py [name ...]"""
import importlib
import pathlib
import sys

here = pathlib.Path(__file__).resolve().parent
public = here.parent / 'public'
sys.path.insert(0, str(here))
sys.dont_write_bytecode = True  # keep __pycache__ out of the docs tree
names = sys.argv[1:] or sorted(p.stem for p in here.glob('*.py') if p.stem not in ('svgkit', 'build'))
for name in names:
    module = importlib.import_module(name)
    for file, diagram in getattr(module, 'diagrams', {name: getattr(module, 'd', None)}).items():
        target = public / (file if '/' in file else f'architecture/{file}')
        target = target.with_name(target.name + '.svg')
        target.parent.mkdir(exist_ok=True)
        target.write_text(diagram.render(), encoding='utf8')
        print(f'wrote {file}.svg')
