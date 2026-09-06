"""Encrypt an original test PDF. Requires pypdf; never use on private originals."""
import argparse
from pathlib import Path
from pypdf import PdfReader, PdfWriter

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('source', type=Path)
parser.add_argument('destination', type=Path)
args = parser.parse_args()
if args.source.resolve() == args.destination.resolve():
    parser.error('Choose a separate output file.')
writer = PdfWriter()
for page in PdfReader(args.source).pages:
    writer.add_page(page)
writer.encrypt('readero-fixture-password')
with args.destination.open('wb') as output:
    writer.write(output)
