#!/usr/bin/env python3
"""
PDF Merge Script using PyPDF2
รองรับภาษาไทยและ fonts ทั้งหมด 100%
"""
import sys
from pypdf import PdfWriter, PdfReader

def merge_pdfs(input_files, output_file):
    """Merge multiple PDF files into one"""
    writer = PdfWriter()
    
    for pdf_file in input_files:
        reader = PdfReader(pdf_file)
        for page in reader.pages:
            writer.add_page(page)
    
    with open(output_file, 'wb') as output:
        writer.write(output)

if __name__ == '__main__':
    if len(sys.argv) < 3:
        print("Usage: merge_pdf.py output.pdf input1.pdf input2.pdf ...")
        sys.exit(1)
    
    output_file = sys.argv[1]
    input_files = sys.argv[2:]
    
    merge_pdfs(input_files, output_file)
    print(f"✅ Merged {len(input_files)} PDFs into {output_file}")

