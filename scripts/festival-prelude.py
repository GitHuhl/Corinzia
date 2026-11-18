#!/usr/bin/env python3
"""Writes "A Festival Prelude", Corinzia's orchestral demo piece, as MusicXML.

An original piece for the app's demo: long and short string writing,
woodwind solos, brass fanfares and chorale, for a full orchestra.

    python3 scripts/festival-prelude.py > assets/demo/festival-prelude.musicxml

Each part is a list of bars; a bar is a list of events
(pitch, sixteenths, marks...). Pitch: "D5", "F#4+A4" (a chord), "r".
Marks: "(" ")" slur start/stop, "." staccato, ">" accent, "-" tenuto,
"~" tremolo (three strokes), "^" fermata, a dynamic ("p", "ff"...),
"<" ">>" "|" crescendo / diminuendo / end of hairpin (before the note),
"T:text" technique words above the staff ("pizz.", "arco").
"""
import sys
from xml.sax.saxutils import escape

STEPS = {"C": 0, "D": 2, "E": 4, "F": 5, "G": 7, "A": 9, "B": 11}
TYPES = {1: ("16th", 0), 2: ("eighth", 0), 3: ("eighth", 1), 4: ("quarter", 0), 6: ("quarter", 1), 8: ("half", 0), 12: ("half", 1), 16: ("whole", 0)}
DYNAMICS = {"ppp", "pp", "p", "mp", "mf", "f", "ff", "fff", "sfz", "fp"}

BARS = 40
# (bar, bpm, words), rehearsal marks
TEMPI = {1: (63, "Andante sostenuto"), 9: (63, None), 17: (144, "Allegro vivace"), 33: (72, "Maestoso")}
REHEARSAL = {9: "A", 17: "B", 25: "C", 33: "D"}

W = lambda p, *m: [(p, 16) + m]  # a whole bar of one event
R = [("r", 16)]


def bars(n, bar=None):
    return [list(bar or R) for _ in range(n)]


# ----------------------------------------------------------------- the music --
# I. Andante (1-8): the theme in the first violins, the strings sustained.
theme = [
    [("F#5", 8, "(", "p"), ("E5", 4), ("D5", 4)],
    [("B5", 12), ("A5", 4, ")")],
    [("G5", 4, "(", "<"), ("F#5", 4), ("E5", 4), ("A5", 4)],
    [("F#5", 16, ")", "|")],
    [("D6", 8, "(", "mf"), ("C#6", 4), ("B5", 4)],
    [("A5", 4), ("G5", 4), ("F#5", 4), ("B5", 4, ")")],
    [("A5", 8, "(", ">>"), ("G5", 4), ("E5", 4)],
    [("D5", 16, ")", "|", "p")],
]
vln2_a = [W("A4", "p"), W("B4"), [("G4", 8, "<"), ("G4", 8)], W("A4", "|"), W("F#4", "mf"), W("G4"), [("E4", 8, ">>"), ("G4", 8)], W("F#4", "|", "p")]
vla_a = [W("D4", "p"), W("D4"), [("B3", 8, "<"), ("C#4", 8)], W("A3", "|"), W("D4", "mf"), W("D4"), [("C#4", 8, ">>"), ("C#4", 8)], W("A3", "|", "p")]
vc_a = [W("D3", "p"), W("G2"), [("E3", 8, "<"), ("A2", 8)], [("D3", 8, "|"), ("F#3", 8)], W("B2", "mf"), W("G2"), [("A2", 8, ">>"), ("A2", 8)], W("D3", "|", "p")]
cb_a = [W("D2", "p"), W("G1"), [("E2", 8, "<"), ("A1", 8)], [("D2", 8, "|"), ("F#2", 8)], W("B1", "mf"), W("G1"), [("A1", 8, ">>"), ("A1", 8)], W("D2", "|", "p")]
hn_a = bars(4) + [W("D4+F#4", "pp", "<"), W("D4+G4"), W("C#4+E4", "|", ">>"), W("A3+D4", "|", "pp")]

# II. Woodwinds (9-16): the oboe sings, the flute answers, pizzicato strings.
ob_b = [
    [("A4", 4, "(", "mf"), ("D5", 4), ("F#5", 8)],
    [("E5", 6), ("D5", 2), ("C#5", 8, ")")],
    [("D5", 4, "("), ("E5", 4), ("F#5", 4), ("G5", 4)],
    [("A5", 12, ")"), ("r", 4)],
] + bars(4)
fl_b = bars(4) + [
    [("A5", 4, "(", "mf"), ("D6", 4), ("F#6", 8)],
    [("E6", 6), ("D6", 2), ("C#6", 8, ")")],
    [("B5", 4, "(", ">>"), ("A5", 4), ("G5", 4), ("E5", 4)],
    [("D5", 12, ")", "|", "p"), ("r", 4)],
]
cl_b = [W("D4+F#4", "p"), W("E4+G4"), [("D4+F#4", 8), ("E4+G4", 8)], W("C#4+E4"), W("F#4+A4", "p"), W("E4+G4"), [("G5", 4, "(", "mp"), ("F#5", 4), ("E5", 4), ("C#5", 4)], [("D5", 12, ")"), ("r", 4)]]
bn_b = [W("D3", "p"), W("A2"), [("D3", 8), ("B2", 8)], W("A2"), W("D3"), W("A2"), [("G2", 8), ("A2", 8)], [("D3", 12), ("r", 4)]]
pizz = lambda a, b, first=False: [(a, 4, "T:pizz.", "p") if first else (a, 4), ("r", 4), (b, 4), ("r", 4)]
vc_b = [pizz("D3", "A2", True), pizz("A2", "E3"), pizz("D3", "B2"), pizz("A2", "C#3"), pizz("D3", "A2"), pizz("A2", "E3"), pizz("G2", "A2"), [("D3", 4), ("r", 12)]]
cb_b = [pizz("D2", "A1", True), pizz("A1", "E2"), pizz("D2", "B1"), pizz("A1", "C#2"), pizz("D2", "A1"), pizz("A1", "E2"), pizz("G1", "A1"), [("D2", 4), ("r", 12)]]
after = lambda c, first=False: [("r", 4), (c, 4, "T:pizz.", "p") if first else (c, 4), ("r", 4), (c, 4)]
vln2_b = bars(4) + [after("F#4+A4", True), after("E4+G4"), after("D4+G4"), [("r", 4), ("F#4+A4", 4), ("r", 8)]]
vla_b = bars(4) + [after("D4", True), after("C#4"), after("B3"), [("r", 4), ("D4", 4), ("r", 8)]]
hn_b = bars(4) + [W("D4", "pp"), W("C#4"), W("B3"), [("A3", 12), ("r", 4)]]


# III. Allegro (17-32): short strings, fanfares, runs, a crescendo.
def run(notes, *first):
    """Sixteen sixteenths, staccato; marks on the first."""
    return [(n, 1, ".") + (tuple(first) if i == 0 else ()) for i, n in enumerate(notes)]


D_UP = "D5 E5 F#5 G5 A5 G5 F#5 E5 D5 E5 F#5 G5 A5 B5 C#6 D6".split()
G_UP = "G5 A5 B5 C#6 D6 C#6 B5 A5 G5 A5 B5 C#6 D6 E6 F#6 G6".split()
A_UP = "A5 B5 C#6 D6 E6 D6 C#6 B5 A5 B5 C#6 D6 E6 F#6 G6 A6".split()
down = lambda notes, o: [n[:-1] + str(int(n[-1]) + o) for n in notes]
eighths = lambda p, *m: [(p, 2, ".") + (m if i == 0 else ()) for i in range(8)]
fanfare = lambda c, *m: [(c, 2, ">") + m, (c, 1, "."), (c, 1, "."), (c, 4, ">"), (c, 4, ">"), ("r", 4)]
hits = lambda c, *m: [(c, 4, ">", ".") + m, ("r", 4), (c, 4, ">", "."), ("r", 4)]

vln1_c = [run(D_UP, "f"), hits("D6"), run(G_UP), hits("C#6"),
          eighths("A5", "mf"), eighths("G5"), eighths("F#5", "<"), eighths("E5"),
          run(D_UP, "f"), hits("D6"), run(A_UP), hits("E6"),
          [("A5", 16, "~", "p", "<")], [("A5", 16, "~")], [("A5", 16, "~", "|", "ff")], [("D6", 4, ">", ".", "ff"), ("r", 8), ("r", 4, "^")]]
vln2_c = [run(down(D_UP, -1), "f", "T:arco"), hits("F#5"), run(down(G_UP, -1)), hits("A5"),
          eighths("F#5", "mf"), eighths("E5"), eighths("D5", "<"), eighths("C#5"),
          run(down(D_UP, -1), "f"), hits("A5"), run(down(A_UP, -1)), hits("G5"),
          [("E5", 16, "~", "p", "<")], [("E5", 16, "~")], [("E5", 16, "~", "|", "ff")], [("F#5", 4, ">", ".", "ff"), ("r", 8), ("r", 4, "^")]]
vla_c = [eighths("D4", "f", "T:arco"), hits("A4"), eighths("D4"), hits("A4"),
         run("D3 E3 F#3 G3 A3 B3 C#4 D4 E4 F#4 G4 A4 B4 C#5 D5 E5".split(), "mf"), run("C#5 B4 A4 G4 F#4 E4 D4 C#4 B3 A3 G3 F#3 E3 D3 C#3 E3".split()), run("D3 F#3 A3 D4 F#4 A4 D5 A4 F#4 D4 A3 F#3 D3 F#3 A3 D4".split(), "<"), run("E3 G3 B3 E4 G4 B4 E5 B4 G4 E4 B3 G3 E3 G3 B3 E4".split()),
         eighths("D4", "f"), hits("A4"), eighths("C#4"), hits("C#4"),
         [("C#4", 16, "~", "p", "<")], [("C#4", 16, "~")], [("C#4", 16, "~", "|", "ff")], [("D4", 4, ">", ".", "ff"), ("r", 8), ("r", 4, "^")]]
vc_c = [eighths("D3", "f", "T:arco"), hits("D3"), eighths("G2"), hits("A2"),
        run("D2 E2 F#2 G2 A2 B2 C#3 D3 E3 F#3 G3 A3 B3 C#4 D4 E4".split(), "mf"), run("C#4 B3 A3 G3 F#3 E3 D3 C#3 B2 A2 G2 F#2 E2 D2 C#2 E2".split()), run("D2 F#2 A2 D3 F#3 A3 D4 A3 F#3 D3 A2 F#2 D2 F#2 A2 D3".split(), "<"), run("E2 G2 B2 E3 G3 B3 E4 B3 G3 E3 B2 G2 E2 G2 B2 E3".split()),
        eighths("D3", "f"), hits("D3"), eighths("A2"), hits("A2"),
        [("A2", 16, "~", "p", "<")], [("A2", 16, "~")], [("A2", 16, "~", "|", "ff")], [("D3", 4, ">", ".", "ff"), ("r", 8), ("r", 4, "^")]]
cb_c = [eighths("D2", "f", "T:arco"), hits("D2"), eighths("G1"), hits("A1"),
        eighths("D2", "mf"), eighths("A1"), eighths("D2", "<"), eighths("E2"),
        eighths("D2", "f"), hits("D2"), eighths("A1"), hits("A1"),
        [("A1", 16, "~", "p", "<")], [("A1", 16, "~")], [("A1", 16, "~", "|", "ff")], [("D2", 4, ">", ".", "ff"), ("r", 8), ("r", 4, "^")]]
fl_c = [R, R, R, R, run(D_UP, "f"), run(down(G_UP, 0)), run(D_UP, "<"), run(down(A_UP, 0)),
        R, [("D6+F#6", 4, ">", "ff"), ("r", 12)], R, [("C#6+E6", 4, ">"), ("r", 12)],
        [("A5+C#6", 16, "p", "<")], W("A5+E6"), W("C#6+E6", "|", "ff"), [("D6+F#6", 4, ">", ".", "ff"), ("r", 8), ("r", 4, "^")]]
ob_c = [R, R, R, R, run(down(D_UP, 0), "mf"), run(down(G_UP, -1)), run(down(D_UP, 0), "<"), run(down(A_UP, -1)),
        R, [("A5+D5", 4, ">", "ff"), ("r", 12)], R, [("A5+C#5", 4, ">"), ("r", 12)],
        [("E5+A5", 16, "p", "<")], W("E5+A5"), W("E5+G5", "|", "ff"), [("F#5+A5", 4, ">", ".", "ff"), ("r", 8), ("r", 4, "^")]]
cl_c = [R, R, R, R, run(down(D_UP, -1), "mf"), run(down(G_UP, -1)), run(down(D_UP, -1), "<"), run(down(A_UP, -1)),
        R, [("F#4+A4", 4, ">", "ff"), ("r", 12)], R, [("E4+G4", 4, ">"), ("r", 12)],
        [("C#5+E5", 16, "p", "<")], W("C#5+E5"), W("C#5+E5", "|", "ff"), [("D5+F#5", 4, ">", ".", "ff"), ("r", 8), ("r", 4, "^")]]
bn_c = [eighths("D3", "f"), hits("D3"), eighths("G2"), hits("A2"),
        R, R, R, R,
        eighths("D3", "f"), hits("D3"), eighths("A2"), hits("A2"),
        [("A2+E3", 16, "p", "<")], W("A2+E3"), W("A2+G3", "|", "ff"), [("D3+A3", 4, ">", ".", "ff"), ("r", 8), ("r", 4, "^")]]
tpt_c = [R, fanfare("D5+F#5", "f"), R, fanfare("C#5+E5"), R, R, R, R,
         R, fanfare("D5+F#5", "ff"), R, fanfare("C#5+E5"),
         [("A4+C#5", 16, "p", "<")], W("A4+C#5"), W("A4+E5", "|", "ff"), [("D5+F#5", 4, ">", ".", "ff"), ("r", 8), ("r", 4, "^")]]
hn_c = [[("D4+A4", 8, ">", "f"), ("r", 8)], fanfare("A3+D4"), [("D4+G4", 8, ">"), ("r", 8)], fanfare("A3+C#4"),
        W("D4+F#4", "p", "<"), W("D4+G4"), W("D4+F#4"), W("C#4+E4", "|", "mf"),
        [("D4+A4", 8, ">", "f"), ("r", 8)], fanfare("A3+D4", "ff"), [("C#4+E4", 8, ">"), ("r", 8)], fanfare("A3+C#4"),
        [("A3+E4", 16, "p", "<")], W("A3+E4"), W("C#4+G4", "|", "ff"), [("D4+F#4", 4, ">", ".", "ff"), ("r", 8), ("r", 4, "^")]]
trb_c = [R, hits("D3+F#3+A3", "f"), R, hits("A2+E3+G3"),
         W("D3+F#3+A3", "pp", "<"), W("D3+G3+B3"), W("D3+F#3+A3"), W("C#3+E3+G3", "|", "mf"),
         R, hits("D3+F#3+A3", "ff"), R, hits("A2+E3+G3"),
         [("A2+E3+G3", 16, "p", "<")], W("A2+E3+G3"), W("A2+E3+G3", "|", "ff"), [("D3+F#3+A3", 4, ">", ".", "ff"), ("r", 8), ("r", 4, "^")]]
tba_c = [R, hits("D2", "f"), R, hits("A1"), W("D2", "pp", "<"), W("G1"), W("D2"), W("A1", "|", "mf"),
         R, hits("D2", "ff"), R, hits("A1"),
         [("A1", 16, "p", "<")], W("A1"), W("A1", "|", "ff"), [("D2", 4, ">", ".", "ff"), ("r", 8), ("r", 4, "^")]]
timp_c = [[("D3", 4, ">", "f"), ("r", 4), ("A2", 4), ("r", 4)], hits("D3"), [("D3", 4, ">"), ("r", 12)], hits("A2"),
          R, R, R, R,
          [("D3", 4, ">", "f"), ("r", 4), ("A2", 4), ("r", 4)], hits("D3", "ff"), [("A2", 4, ">"), ("r", 12)], hits("A2"),
          [("A2", 16, "~", "p", "<")], [("A2", 16, "~")], [("A2", 16, "~", "|", "ff")], [("D3", 4, ">", ".", "ff"), ("r", 8), ("r", 4, "^")]]

# IV. Maestoso (33-40): the theme in the brass, the strings soaring.
tpt_d = [
    [("F#5", 8, "ff", "-"), ("E5", 4, "-"), ("D5", 4, "-")],
    [("B4", 12, "-"), ("A4", 4, "-")],
    [("G4", 4, "-"), ("F#4", 4, "-"), ("E4", 4, "-"), ("A4", 4, "-")],
    [("F#4", 16)],
    [("D5", 8, "-"), ("C#5", 4, "-"), ("B4", 4, "-")],
    [("A4", 4, "<"), ("G4", 4), ("E4+G4", 4), ("E4+A4", 4, "|")],
    [("F#4+A4", 8, "fff"), ("E4+G4", 8)],
    [("F#4+D5", 16, "^", "~")],
]
hn_d = [W("A3+D4", "ff"), [("G3+D4", 12), ("A3+D4", 4)], [("B3+E4", 8), ("C#4+E4", 8)], W("A3+D4"), W("B3+D4"), [("G3+B3", 8, "<"), ("A3+C#4", 8, "|")], [("A3+D4", 8, "fff"), ("A3+C#4", 8)], W("A3+D4+F#4", "^")]
trb_d = [W("D3+F#3+A3", "ff"), W("G2+D3+B3"), [("E3+G3+B3", 8), ("A2+E3+G3", 8)], W("D3+F#3+A3"), W("B2+F#3+B3"), [("G2+D3+B3", 8, "<"), ("A2+E3+G3", 8, "|")], [("D3+F#3+A3", 8, "fff"), ("A2+E3+G3", 8)], W("D3+F#3+A3", "^")]
tba_d = [W("D2", "ff"), W("G1"), [("E2", 8), ("A1", 8)], W("D2"), W("B1"), [("G1", 8, "<"), ("A1", 8, "|")], [("D2", 8, "fff"), ("A1", 8)], W("D2", "^")]
vln1_d = [[("D6", 8, "(", "ff"), ("C#6", 8)], [("B5", 8), ("D6", 8, ")")], [("E6", 8, "("), ("C#6", 8, ")")], [("D6", 4, "("), ("A5", 4), ("F#5", 4), ("A5", 4, ")")],
          [("F#6", 8, "("), ("E6", 8)], [("D6", 8, "<"), ("E6", 8, ")", "|")], [("F#6", 8, "fff"), ("E6", 8)], W("D6", "^")]
vln2_d = [W("A5", "~", "ff"), W("G5", "~"), [("G5", 8, "~"), ("E5", 8, "~")], W("F#5", "~"), W("F#5", "~"), [("G5", 8, "~", "<"), ("A5", 8, "~", "|")], [("A5", 8, "~", "fff"), ("G5", 8, "~")], W("F#5", "~", "^")]
vla_d = [W("F#4", "~", "ff"), W("D4", "~"), [("B3", 8, "~"), ("C#4", 8, "~")], W("D4", "~"), W("D4", "~"), [("D4", 8, "~", "<"), ("E4", 8, "~", "|")], [("D4", 8, "~", "fff"), ("C#4", 8, "~")], W("D4", "~", "^")]
vc_d = [[("D3", 4, "-", "ff"), ("F#3", 4, "-"), ("A3", 4, "-"), ("D4", 4, "-")], [("G2", 4, "-"), ("B2", 4, "-"), ("D3", 4, "-"), ("G3", 4, "-")], [("E3", 8), ("A2", 8)], [("D3", 4, "-"), ("F#3", 4, "-"), ("A3", 4, "-"), ("F#3", 4, "-")],
        [("B2", 4, "-"), ("D3", 4, "-"), ("F#3", 4, "-"), ("B3", 4, "-")], [("G2", 8, "<"), ("A2", 8, "|")], [("D3", 8, "fff"), ("A2", 8)], W("D3", "^")]
cb_d = [W("D2", "ff"), W("G1"), [("E2", 8), ("A1", 8)], W("D2"), W("B1"), [("G1", 8, "<"), ("A1", 8, "|")], [("D2", 8, "fff"), ("A1", 8)], W("D2", "^")]
fl_d = [W("F#6+A6", "ff"), W("D6+G6"), [("E6+G6", 8), ("E6+A6", 8)], W("F#6+A6"), W("D6+F#6"), [("D6+G6", 8, "<"), ("E6+A6", 8, "|")], [("F#6+A6", 8, "fff"), ("E6+G6", 8)], W("F#6+A6", "^")]
ob_d = [W("A5+D6", "ff"), W("G5+B5"), [("G5+B5", 8), ("A5+C#6", 8)], W("A5+D6"), W("F#5+B5"), [("G5+B5", 8, "<"), ("A5+C#6", 8, "|")], [("A5+D6", 8, "fff"), ("A5+C#6", 8)], W("A5+D6", "^")]
cl_d = [W("D5+F#5", "ff"), W("D5+G5"), [("E5+G5", 8), ("E5+A5", 8)], W("D5+F#5"), W("D5+F#5"), [("D5+G5", 8, "<"), ("E5+A5", 8, "|")], [("D5+F#5", 8, "fff"), ("C#5+E5", 8)], W("D5+F#5", "^")]
bn_d = [W("D3+A3", "ff"), W("G2+D3"), [("E3+B3", 8), ("A2+E3", 8)], W("D3+A3"), W("B2+F#3"), [("G2+D3", 8, "<"), ("A2+E3", 8, "|")], [("D3+A3", 8, "fff"), ("A2+E3", 8)], W("D3+A3", "^")]
timp_d = [[("D3", 4, ">", "ff"), ("r", 4), ("D3", 4), ("r", 4)], [("r", 12), ("A2", 4)], R, [("D3", 4, ">"), ("r", 12)], R, [("A2", 16, "~", "<")], [("A2", 16, "~", "|", "fff")], W("D3", "~", "^")]

S8 = bars(8)
S16 = bars(16)
PARTS = [
    ("Flutes", "Fl.", ("G", 2), S8 + fl_b + fl_c + fl_d),
    ("Oboes", "Ob.", ("G", 2), S8 + ob_b + ob_c + ob_d),
    ("Clarinets", "Cl.", ("G", 2), S8 + cl_b + cl_c + cl_d),
    ("Bassoons", "Bsn.", ("F", 4), S8 + bn_b + bn_c + bn_d),
    ("Horns", "Hn.", ("G", 2), hn_a + hn_b + hn_c + hn_d),
    ("Trumpets", "Tpt.", ("G", 2), S8 + bars(8) + tpt_c + tpt_d),
    ("Trombones", "Tbn.", ("F", 4), S8 + bars(8) + trb_c + trb_d),
    ("Tuba", "Tba.", ("F", 4), S8 + bars(8) + tba_c + tba_d),
    ("Timpani", "Timp.", ("F", 4), S8 + bars(8) + timp_c + timp_d),
    ("Violins I", "Vln. I", ("G", 2), theme + bars(8) + vln1_c + vln1_d),
    ("Violins II", "Vln. II", ("G", 2), vln2_a + vln2_b + vln2_c + vln2_d),
    ("Violas", "Vla.", ("C", 3), vla_a + vla_b + vla_c + vla_d),
    ("Cellos", "Vc.", ("F", 4), vc_a + vc_b + vc_c + vc_d),
    ("Basses", "Cb.", ("F", 4), cb_a + cb_b + cb_c + cb_d),
]


# ------------------------------------------------------------------ MusicXML --
def pitch_xml(p):
    step, rest = p[0], p[1:]
    alter = 1 if rest.startswith("#") else -1 if rest.startswith("b") else 0
    octave = int(rest.lstrip("#b"))
    return f"<pitch><step>{step}</step>{f'<alter>{alter}</alter>' if alter else ''}<octave>{octave}</octave></pitch>"


def direction(inner, placement="below"):
    return f'<direction placement="{placement}"><direction-type>{inner}</direction-type></direction>'


def bar_xml(events, index, first_part, clef):
    out = []
    if index == 0:
        out.append(f"<attributes><divisions>4</divisions><key><fifths>2</fifths></key><time><beats>4</beats><beat-type>4</beat-type></time><clef><sign>{clef[0]}</sign><line>{clef[1]}</line></clef></attributes>")
    bar = index + 1
    if first_part and bar in REHEARSAL:
        out.append(direction(f"<rehearsal>{REHEARSAL[bar]}</rehearsal>", "above"))
    if first_part and bar in TEMPI:
        bpm, words = TEMPI[bar]
        inner = (f"<words>{escape(words)}</words>" if words else "") + f"<metronome><beat-unit>quarter</beat-unit><per-minute>{bpm}</per-minute></metronome>"
        out.append(f'<direction placement="above"><direction-type>{inner}</direction-type><sound tempo="{bpm}"/></direction>')
    total = 0
    for ev in events:
        p, dur, marks = ev[0], ev[1], ev[2:]
        total += dur
        for m in marks:
            if m in DYNAMICS:
                out.append(direction(f"<dynamics><{m}/></dynamics>"))
            elif m == "<":
                out.append(direction('<wedge type="crescendo"/>'))
            elif m == ">>":
                out.append(direction('<wedge type="diminuendo"/>'))
            elif m == "|":
                out.append(direction('<wedge type="stop"/>'))
            elif m.startswith("T:"):
                out.append(direction(f"<words>{escape(m[2:])}</words>", "above"))
        typ, dots = TYPES[dur]
        notations = []
        arts = [a for a, k in (("staccato", "."), ("accent", ">"), ("tenuto", "-")) if k in marks]
        if arts:
            notations.append("<articulations>" + "".join(f"<{a}/>" for a in arts) + "</articulations>")
        if "(" in marks:
            notations.append('<slur type="start" number="1"/>')
        if ")" in marks:
            notations.append('<slur type="stop" number="1"/>')
        if "^" in marks:
            notations.append("<fermata/>")
        if "~" in marks:
            notations.append('<ornaments><tremolo type="single">3</tremolo></ornaments>')
        nxml = f"<notations>{''.join(notations)}</notations>" if notations else ""
        tail = f"<duration>{dur}</duration><voice>1</voice><type>{typ}</type>" + "<dot/>" * dots
        if p == "r":
            whole = ' measure="yes"' if dur == 16 else ""
            out.append(f"<note><rest{whole}/>{tail}{nxml}</note>")
        else:
            for i, q in enumerate(p.split("+")):
                chord = "<chord/>" if i else ""
                out.append(f"<note>{chord}{pitch_xml(q)}{tail}{nxml if i == 0 else ''}</note>")
    assert total == 16, f"bar {bar}: {total} sixteenths in {events}"
    if bar == BARS:
        out.append('<barline location="right"><bar-style>light-heavy</bar-style></barline>')
    return f'<measure number="{bar}">{"".join(out)}</measure>'


def main():
    xml = ['<?xml version="1.0" encoding="UTF-8"?>',
           '<score-partwise version="3.1">',
           "<work><work-title>A Festival Prelude</work-title></work>",
           '<identification><creator type="composer">Corinzia</creator><rights>Public domain (CC0)</rights></identification>',
           "<part-list>"]
    for i, (name, abbr, _, _) in enumerate(PARTS):
        xml.append(f'<score-part id="P{i + 1}"><part-name>{escape(name)}</part-name><part-abbreviation>{escape(abbr)}</part-abbreviation></score-part>')
    xml.append("</part-list>")
    for i, (name, _, clef, music) in enumerate(PARTS):
        assert len(music) == BARS, f"{name}: {len(music)} bars"
        xml.append(f'<part id="P{i + 1}">')
        xml.extend(bar_xml(b, k, i == 0, clef) for k, b in enumerate(music))
        xml.append("</part>")
    xml.append("</score-partwise>")
    sys.stdout.write("\n".join(xml) + "\n")


main()
