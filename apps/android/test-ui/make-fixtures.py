#!/usr/bin/env python3
"""Writes the PDFs the UI tests open into assets/.

    python3 make-fixtures.py

The files are checked in. Rerun this only to change them. They are plain,
uncompressed PDFs using the standard fonts, so they stay small and diff-able.

pages.pdf  12 A4 pages, each headed "Page n of 12".
book.pdf   14 pages with a nested outline, a linked table of contents on
           page 1 (plus one link to a web page), "Back to contents" links,
           printed page numbers i-iii, 1-9, A-1, A-2, and a title, an author
           and a creation date.
locked.pdf 3 pages, encrypted (RC4, 40 bit) with the password "secret".
slow.pdf   2 pages. The first takes long to draw (about a quarter of a second
           for each part of it on a fast machine): 768,000 small curves, and
           its resources name a 120 megapixel image, which marks it as a page
           to be patient with from the start. Nothing draws the image.
form.pdf   2 pages with a form on the first: a text field "name", which the
           form calls "Your name", at x 150 to 450, y 690 to 712, a text field
           "code" of at most 4 characters at x 150 to 250, y 650 to 672, a
           checkbox "agree" at x 150 to 170, y 610 to 630, and a menu
           "country" at x 150 to 350, y 570 to 592, which offers Germany,
           France and Other and holds Germany, a menu "color" at x 150 to 350,
           y 530 to 552, which offers Red and Green and takes text of one's
           own, and a list "days" at x 150 to 350, y 450 to 510, which offers
           Mon, Tue and Wed and takes several of them. Reset, "name" holds
           "Nobody".
"""

import hashlib
from pathlib import Path

OUT = Path(__file__).parent / "assets"
FONTS = {
    5: b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica-Bold >>",
    6: b"<< /Type /Font /Subtype /Type1 /BaseFont /Times-Roman >>",
}
RESOURCES = "/Resources << /Font << /F1 5 0 R /F2 6 0 R >> >>"
LINE = "the quick brown fox jumps over the lazy dog and keeps running along the page"


def write(name, objects, trailer_root=1, trailer_extra=""):
    """Serializes numbered objects with a correct cross-reference table."""
    out = bytearray(b"%PDF-1.7\n")
    offsets = {}
    for number in sorted(objects):
        offsets[number] = len(out)
        out += b"%d 0 obj\n" % number + objects[number] + b"\nendobj\n"
    size = max(objects) + 1
    xref = len(out)
    out += b"xref\n0 %d\n" % size
    for number in range(size):
        if number in offsets:
            out += b"%010d 00000 n \n" % offsets[number]
        else:
            out += b"0000000000 65535 f \n"
    out += b"trailer\n<< /Size %d /Root %d 0 R %s >>\nstartxref\n%d\n%%%%EOF\n" % (
        size,
        trailer_root,
        trailer_extra.encode(),
        xref,
    )
    (OUT / name).write_bytes(out)
    print(f"{name}: {len(out)} bytes")


def stream(content):
    data = content.encode()
    return b"<< /Length %d >>\nstream\n" % len(data) + data + b"endstream"


def page_id(page):
    """Object number of 1-based page `page`. Its content stream follows it."""
    return 10 + 2 * (page - 1)


def heading(page, count):
    return (
        "0.15 0.35 0.75 rg 50 760 495 40 re f\n"
        f"1 1 1 rg BT /F1 22 Tf 62 772 Td (Page {page} of {count}) Tj ET\n"
        "0.1 0.1 0.1 rg\n"
    )


def body():
    lines = "".join(f"({LINE}) Tj T*\n" for _ in range(40))
    return "BT /F2 11 Tf 50 735 Td 15 TL\n" + lines + "ET\n"


def pages_pdf(count=12):
    objects = dict(FONTS)
    objects[1] = b"<< /Type /Catalog /Pages 2 0 R >>"
    kids = " ".join(f"{page_id(p)} 0 R" for p in range(1, count + 1))
    objects[2] = f"<< /Type /Pages /Kids [{kids}] /Count {count} >>".encode()
    for p in range(1, count + 1):
        objects[page_id(p)] = (
            f"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] "
            f"/Contents {page_id(p) + 1} 0 R {RESOURCES} >>"
        ).encode()
        objects[page_id(p) + 1] = stream(heading(p, count) + body())
    write("pages.pdf", objects)


def book_pdf(count=14):
    chapters = [
        ("1 Getting started", 1, [("1.1 Installing", 2), ("1.2 First document", 3)]),
        ("2 Viewing", 5, [("2.1 Zoom and scroll", 6), ("2.2 Pages and contents", 7), ("2.3 Search", 9)]),
        ("3 Editing", 10, [("3.1 Annotations", 11)]),
        ("Appendix", 13, []),
    ]

    def dest(page):
        return f"[{page_id(page)} 0 R /XYZ 0 842 0]"

    objects = dict(FONTS)
    # Outline: chapters from object 100, each followed by its sections.
    number = 100
    top = []
    for title, page, sections in chapters:
        me = number
        number += 1
        kids = []
        for section_title, section_page in sections:
            kids.append((number, section_title, section_page))
            number += 1
        top.append((me, title, page, kids))
    for i, (me, title, page, kids) in enumerate(top):
        entry = f"<< /Title ({title}) /Parent 99 0 R /Dest {dest(page)}"
        if i > 0:
            entry += f" /Prev {top[i - 1][0]} 0 R"
        if i < len(top) - 1:
            entry += f" /Next {top[i + 1][0]} 0 R"
        if kids:
            entry += f" /First {kids[0][0]} 0 R /Last {kids[-1][0]} 0 R /Count {len(kids)}"
        objects[me] = (entry + " >>").encode()
        for k, (kid, section_title, section_page) in enumerate(kids):
            entry = f"<< /Title ({section_title}) /Parent {me} 0 R /Dest {dest(section_page)}"
            if k > 0:
                entry += f" /Prev {kids[k - 1][0]} 0 R"
            if k < len(kids) - 1:
                entry += f" /Next {kids[k + 1][0]} 0 R"
            objects[kid] = (entry + " >>").encode()
    objects[99] = f"<< /Type /Outlines /First {top[0][0]} 0 R /Last {top[-1][0]} 0 R /Count {len(top)} >>".encode()

    labels = "/PageLabels << /Nums [0 << /S /r >> 3 << /S /D >> 12 << /P (A-) /S /D >>] >>"
    objects[1] = f"<< /Type /Catalog /Pages 2 0 R /Outlines 99 0 R {labels} >>".encode()
    kids = " ".join(f"{page_id(p)} 0 R" for p in range(1, count + 1))
    objects[2] = f"<< /Type /Pages /Kids [{kids}] /Count {count} >>".encode()

    contents = []
    for title, page, sections in chapters:
        contents.append((title, page, 0))
        contents += [(t, p, 1) for t, p in sections]

    annotation = 200
    for p in range(1, count + 1):
        content = heading(p, count)
        annotations = []
        if p == 1:
            y = 700
            content += "BT /F1 16 Tf 50 720 Td (Contents) Tj ET\n"
            for title, target, indent in contents:
                y -= 28
                x = 60 + indent * 24
                content += f"0 0 0.7 rg BT /F2 14 Tf {x} {y} Td ({title} ........ {target}) Tj ET\n"
                objects[annotation] = (
                    f"<< /Type /Annot /Subtype /Link /Border [0 0 0] "
                    f"/Rect [{x - 5} {y - 6} 500 {y + 16}] /Dest {dest(target)} >>"
                ).encode()
                annotations.append(annotation)
                annotation += 1
            y -= 50
            content += f"0 0 0.7 rg BT /F2 14 Tf 60 {y} Td (External link: https://example.com/specto) Tj ET\n"
            objects[annotation] = (
                f"<< /Type /Annot /Subtype /Link /Border [0 0 0] /Rect [55 {y - 6} 500 {y + 16}] "
                f"/A << /S /URI /URI (https://example.com/specto) >> >>"
            ).encode()
            annotations.append(annotation)
            annotation += 1
        else:
            content += body() + "0 0 0.7 rg BT /F2 12 Tf 50 60 Td (Back to contents) Tj ET\n"
            objects[annotation] = (
                f"<< /Type /Annot /Subtype /Link /Border [0 0 0] /Rect [45 52 200 76] /Dest {dest(1)} >>"
            ).encode()
            annotations.append(annotation)
            annotation += 1
        refs = " ".join(f"{a} 0 R" for a in annotations)
        objects[page_id(p)] = (
            f"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] /Annots [{refs}] "
            f"/Contents {page_id(p) + 1} 0 R {RESOURCES} >>"
        ).encode()
        objects[page_id(p) + 1] = stream(content)
    # What the document says about itself, for "Properties".
    info = max(objects) + 1
    objects[info] = (
        b"<< /Title (The Specto handbook) /Author (Ada Lovelace) /Creator (make-fixtures.py) "
        b"/CreationDate (D:20240305142210+01'00') >>"
    )
    write("book.pdf", objects, trailer_extra=f"/Info {info} 0 R")


def slow_pdf(curves=400, cells=48, rows=40):
    def form(extra, content):
        data = content.encode()
        head = "<< /Type /XObject /Subtype /Form %s /Length %d >>\nstream\n" % (extra, len(data))
        return head.encode() + data + b"endstream"

    # A cell of curves, a row of cells, and a page of rows: a small file
    # with a lot to draw.
    cell = "0.2 w\n" + "".join(
        f"{i % 20} {i * 7 % 20} m {i * 3 % 20} {i * 11 % 20} "
        f"{i * 5 % 20} {i * 13 % 20} {i * 17 % 20} {i * 19 % 20} c S\n"
        for i in range(curves)
    )
    row = "".join(f"q 1 0 0 1 {50 + j % 24 * 20} 0 cm /Cell Do Q\n" for j in range(cells))
    page = "0.3 0.3 0.6 RG\n" + "".join(
        f"q 1 0 0 1 0 {30 + r % 36 * 20} cm /Row Do Q\n" for r in range(rows)
    )
    objects = dict(FONTS)
    objects[1] = b"<< /Type /Catalog /Pages 2 0 R >>"
    objects[2] = b"<< /Type /Pages /Kids [10 0 R 12 0 R] /Count 2 /MediaBox [0 0 595 842] >>"
    objects[7] = form("/BBox [0 0 20 20]", cell)
    objects[8] = form("/BBox [0 0 595 20] /Resources << /XObject << /Cell 7 0 R >> >>", row)
    objects[9] = (
        b"<< /Type /XObject /Subtype /Image /Width 12000 /Height 10000"
        b" /ColorSpace /DeviceGray /BitsPerComponent 1 /Length 0 >>\nstream\n\nendstream"
    )
    objects[10] = (
        b"<< /Type /Page /Parent 2 0 R /Contents 11 0 R /Resources <<"
        b" /Font << /F1 5 0 R /F2 6 0 R >> /XObject << /Row 8 0 R /Big 9 0 R >> >> >>"
    )
    objects[11] = stream(page + heading(1, 2))
    objects[12] = f"<< /Type /Page /Parent 2 0 R {RESOURCES} /Contents 13 0 R >>".encode()
    objects[13] = stream(heading(2, 2) + body())
    write("slow.pdf", objects)


def rc4(key, data):
    box = list(range(256))
    j = 0
    for i in range(256):
        j = (j + box[i] + key[i % len(key)]) % 256
        box[i], box[j] = box[j], box[i]
    out = bytearray()
    i = j = 0
    for byte in data:
        i = (i + 1) % 256
        j = (j + box[i]) % 256
        box[i], box[j] = box[j], box[i]
        out.append(byte ^ box[(box[i] + box[j]) % 256])
    return bytes(out)


def locked_pdf(count=3, user_password=b"secret", owner_password=b"owner"):
    """The standard security handler, revision 2 (PDF 1.4, 7.6.3): RC4, 40 bit."""
    padding = bytes.fromhex(
        "28BF4E5E4E758A4164004E56FFFA01082E2E00B6D0683E802F0CA9FE6453697A"
    )
    file_id = hashlib.md5(b"specto locked fixture").digest()
    permissions = (-4).to_bytes(4, "little", signed=True)

    def pad(password):
        return (password + padding)[:32]

    owner = rc4(hashlib.md5(pad(owner_password)).digest()[:5], pad(user_password))
    key = hashlib.md5(pad(user_password) + owner + permissions + file_id).digest()[:5]
    user = rc4(key, padding)

    def encrypted_stream(number, content):
        object_key = hashlib.md5(key + number.to_bytes(3, "little") + b"\0\0").digest()[:10]
        data = rc4(object_key, content.encode())
        return b"<< /Length %d >>\nstream\n" % len(data) + data + b"\nendstream"

    objects = dict(FONTS)
    objects[1] = b"<< /Type /Catalog /Pages 2 0 R >>"
    kids = " ".join(f"{page_id(p)} 0 R" for p in range(1, count + 1))
    objects[2] = f"<< /Type /Pages /Kids [{kids}] /Count {count} >>".encode()
    for p in range(1, count + 1):
        objects[page_id(p)] = (
            f"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] "
            f"/Contents {page_id(p) + 1} 0 R {RESOURCES} >>"
        ).encode()
        objects[page_id(p) + 1] = encrypted_stream(page_id(p) + 1, heading(p, count) + body())
    objects[9] = (
        f"<< /Filter /Standard /V 1 /R 2 /O <{owner.hex()}> /U <{user.hex()}> /P -4 >>"
    ).encode()
    write(
        "locked.pdf",
        objects,
        trailer_extra=f"/Encrypt 9 0 R /ID [<{file_id.hex()}> <{file_id.hex()}>]",
    )


def form_pdf():
    objects = dict(FONTS)
    objects[1] = (
        b"<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [20 0 R 21 0 R 22 0 R 25 0 R 26 0 R 27 0 R] "
        b"/DA (/Helv 0 Tf 0 g) /DR << /Font << /Helv 3 0 R >> >> >> >>"
    )
    objects[2] = b"<< /Type /Pages /Kids [10 0 R 12 0 R] /Count 2 >>"
    objects[3] = (
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica "
        b"/Encoding /WinAnsiEncoding >>"
    )
    fields = {
        "Name": (150, 690, 450, 712),
        "Code": (150, 650, 250, 672),
        "Agree": (150, 610, 170, 630),
        "Country": (150, 570, 350, 592),
        "Color": (150, 530, 350, 552),
        "Days": (150, 450, 350, 510),
    }
    # Each field has its name beside it and a frame around it.
    content = heading(1, 2) + "0.5 G 1 w\n"
    for label, (x0, y0, x1, y1) in fields.items():
        content += f"BT /F2 12 Tf 62 {y0 + 6} Td ({label}) Tj ET\n"
        content += f"{x0} {y0} {x1 - x0} {y1 - y0} re S\n"
    for p in (1, 2):
        annots = "/Annots [20 0 R 21 0 R 22 0 R 25 0 R 26 0 R 27 0 R]" if p == 1 else ""
        objects[page_id(p)] = (
            f"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] "
            f"/Contents {page_id(p) + 1} 0 R {RESOURCES} {annots} >>"
        ).encode()
        objects[page_id(p) + 1] = stream(content if p == 1 else heading(2, 2) + body())

    def widget(kind, name, rect, more=""):
        x0, y0, x1, y1 = rect
        return (
            f"<< /Type /Annot /Subtype /Widget /FT /{kind} /T ({name}) "
            f"/Rect [{x0} {y0} {x1} {y1}] /P 10 0 R /F 4 {more} >>"
        ).encode()

    def look(content):
        data = content.encode()
        return (
            b"<< /Type /XObject /Subtype /Form /BBox [0 0 20 20] /Length %d >>\nstream\n"
            % len(data)
            + data
            + b"endstream"
        )

    objects[20] = widget("Tx", "name", fields["Name"], "/TU (Your name) /DV (Nobody)")
    objects[21] = widget("Tx", "code", fields["Code"], "/MaxLen 4")
    objects[22] = widget(
        "Btn",
        "agree",
        fields["Agree"],
        "/V /Off /AS /Off /AP << /N << /Yes 23 0 R /Off 24 0 R >> >>",
    )
    objects[23] = look("0.1 0.5 0.2 RG 3 w 4 10 m 8 5 l 16 16 l S\n")
    objects[24] = look("")
    objects[25] = widget(
        "Ch",
        "country",
        fields["Country"],
        "/Ff 131072 /V (de) /Opt [[(de) (Germany)] [(fr) (France)] (Other)]",
    )
    objects[26] = widget("Ch", "color", fields["Color"], "/Ff 393216 /Opt [(Red) (Green)]")
    objects[27] = widget("Ch", "days", fields["Days"], "/Ff 2097152 /Opt [(Mon) (Tue) (Wed)]")
    write("form.pdf", objects)


if __name__ == "__main__":
    OUT.mkdir(exist_ok=True)
    pages_pdf()
    book_pdf()
    locked_pdf()
    slow_pdf()
    form_pdf()
