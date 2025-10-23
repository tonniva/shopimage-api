#!/usr/bin/env python3
"""
Remove Background Script using rembg
ลบพื้นหลังรูปภาพอัตโนมัติ + เพิ่มเส้นขอบรอบวัตถุ (optional)
"""
import sys
import argparse
from rembg import remove
from PIL import Image, ImageFilter, ImageChops
import io
import numpy as np

def remove_background(input_path, output_path, border_size=0, border_color="white"):
    """
    Remove background from image และเพิ่มเส้นขอบรอบวัตถุ
    
    Args:
        input_path: ไฟล์รูปภาพ input
        output_path: ไฟล์ output (PNG)
        border_size: ความหนาของเส้นขอบรอบวัตถุ (pixels) - default 0 = ไม่มีขอบ
        border_color: สีของเส้นขอบ (white, black, red, etc.) - default white
    """
    try:
        # อ่านรูปภาพ
        with open(input_path, 'rb') as input_file:
            input_data = input_file.read()
        
        # ลบพื้นหลัง
        output_data = remove(input_data)
        
        # แปลงเป็น PIL Image
        img = Image.open(io.BytesIO(output_data)).convert("RGBA")
        
        # ถ้าต้องการเพิ่มเส้นขอบรอบวัตถุ
        if border_size > 0:
            # แยก alpha channel (ส่วนที่ไม่ใช่พื้นหลัง)
            r, g, b, alpha = img.split()
            
            # สร้าง outline จาก alpha channel
            # ใช้ filter dilate เพื่อขยาย alpha ออกไป
            outline = alpha.copy()
            for _ in range(border_size):
                outline = outline.filter(ImageFilter.MaxFilter(3))
            
            # สร้างเส้นขอบ = outline - alpha (ส่วนที่ขยายออกไป)
            border_mask = ImageChops.subtract(outline, alpha)
            
            # แปลงสีขอบ
            color_rgb = parse_color(border_color)
            
            # สร้าง layer สำหรับเส้นขอบ
            border_layer = Image.new("RGBA", img.size, (0, 0, 0, 0))
            border_pixels = border_layer.load()
            border_mask_pixels = border_mask.load()
            
            # วาดเส้นขอบ
            for y in range(img.size[1]):
                for x in range(img.size[0]):
                    if border_mask_pixels[x, y] > 0:
                        border_pixels[x, y] = (color_rgb[0], color_rgb[1], color_rgb[2], border_mask_pixels[x, y])
            
            # รวม border layer กับรูปต้นฉบับ
            result = Image.alpha_composite(border_layer, img)
            
            # บันทึกผลลัพธ์
            result.save(output_path, "PNG")
        else:
            # บันทึกโดยไม่มีเส้นขอบ
            img.save(output_path, "PNG")
        
        return True
    except Exception as e:
        print(f"Error: {e}", file=sys.stderr)
        return False

def parse_color(color_str):
    """แปลงชื่อสีหรือ RGB string เป็น tuple (R, G, B)"""
    color_map = {
        'white': (255, 255, 255),
        'black': (0, 0, 0),
        'red': (255, 0, 0),
        'green': (0, 255, 0),
        'blue': (0, 0, 255),
        'yellow': (255, 255, 0),
        'cyan': (0, 255, 255),
        'magenta': (255, 0, 255),
        'gray': (128, 128, 128),
        'orange': (255, 165, 0),
        'pink': (255, 192, 203),
        'purple': (128, 0, 128),
    }
    
    color_lower = color_str.lower().strip()
    
    # ถ้าเป็นชื่อสี
    if color_lower in color_map:
        return color_map[color_lower]
    
    # ถ้าเป็น rgb(r, g, b)
    if color_lower.startswith('rgb(') and color_lower.endswith(')'):
        rgb_str = color_lower[4:-1]
        r, g, b = map(int, rgb_str.split(','))
        return (r, g, b)
    
    # ถ้าเป็น hex #RRGGBB
    if color_lower.startswith('#'):
        hex_color = color_lower.lstrip('#')
        r = int(hex_color[0:2], 16)
        g = int(hex_color[2:4], 16)
        b = int(hex_color[4:6], 16)
        return (r, g, b)
    
    # default: white
    return (255, 255, 255)

if __name__ == '__main__':
    parser = argparse.ArgumentParser(description='Remove background from image')
    parser.add_argument('input', help='Input image file')
    parser.add_argument('output', help='Output image file (PNG)')
    parser.add_argument('--border', type=int, default=0, help='Border size in pixels (default: 0)')
    parser.add_argument('--border-color', default='white', help='Border color (default: white)')
    
    args = parser.parse_args()
    
    if remove_background(args.input, args.output, args.border, args.border_color):
        if args.border > 0:
            print(f"✅ Background removed with {args.border}px {args.border_color} border: {args.output}")
        else:
            print(f"✅ Background removed: {args.output}")
        sys.exit(0)
    else:
        print("❌ Failed to remove background", file=sys.stderr)
        sys.exit(1)


