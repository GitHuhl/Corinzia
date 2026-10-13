import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import Corinzia.Core

// Dorico-style left panel: everything needed to write notes with the mouse,
// mirrored by keyboard shortcuts (shown in tooltips).
Pane {
    id: panel
    padding: 6
    implicitWidth: 124

    component PanelButton: ToolButton {
        id: b
        property string tip: ""
        implicitWidth: 34
        implicitHeight: 34
        checkable: false
        focusPolicy: Qt.NoFocus          // typing must keep going to the score
        ToolTip.text: tip
        ToolTip.visible: hovered && tip.length > 0
        ToolTip.delay: 400
        background: Rectangle {
            radius: 4
            color: b.checked ? "#2f6fb5" : (b.down ? "#c9d6e6" : (b.hovered ? "#e3e9f1" : "transparent"))
            border.color: b.checked ? "#1f4f85" : "#c7c7c7"
        }
        palette.buttonText: b.checked ? "white" : "black"
    }
    component SectionLabel: Label {
        Layout.topMargin: 6
        font.pixelSize: 11
        font.bold: true
        opacity: 0.7
    }

    ScrollView {
        anchors.fill: parent
        contentWidth: availableWidth
        ColumnLayout {
            width: parent.width
            spacing: 4

            Button {
                focusPolicy: Qt.NoFocus
                Layout.fillWidth: true
                text: App.inputMode ? "Input ON" : "Input (N)"
                checkable: true
                checked: App.inputMode
                onToggled: App.inputMode = checked
                highlighted: App.inputMode
            }

            SectionLabel { text: "Duration" }
            GridLayout {
                columns: 3
                columnSpacing: 2
                rowSpacing: 2
                Repeater {
                    model: [ { v: 0, k: "7", n: "Whole" }, { v: 1, k: "6", n: "Half" }, { v: 2, k: "5", n: "Quarter" },
                             { v: 3, k: "4", n: "Eighth" }, { v: 4, k: "3", n: "16th" }, { v: 5, k: "2", n: "32nd" },
                             { v: 6, k: "1", n: "64th" } ]
                    delegate: PanelButton {
                        required property var modelData
                        objectName: "duration" + modelData.v
                        checked: App.duration === modelData.v
                        tip: modelData.n + " (" + modelData.k + ")"
                        onClicked: App.duration = modelData.v
                        contentItem: NoteIcon { value: modelData.v; color: parent.checked ? "white" : "black" }
                    }
                }
            }
            RowLayout {
                spacing: 2
                PanelButton {
                    tip: "Dot (.) — click again for double dot"
                    checked: App.dots > 0
                    onClicked: App.dots = (App.dots + 1) % 3
                    contentItem: Label { text: App.dots === 2 ? "••" : "•"; horizontalAlignment: Text.AlignHCenter; verticalAlignment: Text.AlignVCenter; color: parent.checked ? "white" : "black"; font.pixelSize: 16 }
                }
                PanelButton {
                    tip: "Triplet (;)"
                    checked: App.triplet
                    onClicked: App.triplet = !App.triplet
                    contentItem: Label { text: "3"; font.italic: true; font.bold: true; horizontalAlignment: Text.AlignHCenter; verticalAlignment: Text.AlignVCenter; color: parent.checked ? "white" : "black" }
                }
                PanelButton {
                    tip: "Rest (R or ,)"
                    onClicked: App.enterRest()
                    contentItem: NoteIcon { value: App.duration; rest: true }
                }
            }
            RowLayout {
                spacing: 2
                PanelButton {
                    tip: "Chord: add notes to the previous one (Q, or Shift+letter)"
                    checked: App.chordMode
                    onClicked: App.chordMode = !App.chordMode
                    contentItem: Label { text: "♫"; font.pixelSize: 17; horizontalAlignment: Text.AlignHCenter; verticalAlignment: Text.AlignVCenter; color: parent.checked ? "white" : "black" }
                }
                PanelButton {
                    tip: "Tie selected note (T)"
                    onClicked: App.toggleTie()
                    contentItem: Label { text: "‿"; font.pixelSize: 20; horizontalAlignment: Text.AlignHCenter; verticalAlignment: Text.AlignVCenter }
                }
                PanelButton {
                    tip: "Delete selection (Del) — Backspace deletes before the caret"
                    onClicked: App.deleteSelection()
                    contentItem: Label { text: "⌫"; font.pixelSize: 15; horizontalAlignment: Text.AlignHCenter; verticalAlignment: Text.AlignVCenter }
                }
            }

            SectionLabel { text: "Accidental" }
            RowLayout {
                spacing: 2
                Repeater {
                    model: [ { a: -1, t: "♭", k: "-" }, { a: 0, t: "♮", k: "0" }, { a: 1, t: "♯", k: "=" } ]
                    delegate: PanelButton {
                        required property var modelData
                        tip: "Accidental for the next note (" + modelData.k + ")"
                        checked: App.accidental === modelData.a
                        onClicked: App.accidental = checked ? 9 : modelData.a
                        contentItem: Label { text: modelData.t; font.pixelSize: 18; horizontalAlignment: Text.AlignHCenter; verticalAlignment: Text.AlignVCenter; color: parent.checked ? "white" : "black" }
                    }
                }
            }

            SectionLabel { text: "Articulation" }
            GridLayout {
                columns: 3
                columnSpacing: 2
                rowSpacing: 2
                Repeater {
                    // flags: model::Articulation
                    model: [ { f: 1, t: "•", n: "Staccato ([)" }, { f: 2, t: ">", n: "Accent (')" },
                             { f: 4, t: "–", n: "Tenuto (])" }, { f: 8, t: "^", n: "Marcato (\\)" },
                             { f: 16, t: "𝄐", n: "Fermata" } ]
                    delegate: PanelButton {
                        required property var modelData
                        tip: modelData.n + " on the selection"
                        onClicked: App.toggleArticulation(modelData.f)
                        contentItem: Label { text: modelData.t; font.pixelSize: 16; font.bold: true; horizontalAlignment: Text.AlignHCenter; verticalAlignment: Text.AlignVCenter }
                    }
                }
            }

            SectionLabel { text: "Dynamic" }
            GridLayout {
                columns: 3
                columnSpacing: 2
                rowSpacing: 2
                Repeater {
                    model: [ "pp", "p", "mp", "mf", "f", "ff" ]
                    delegate: PanelButton {
                        required property var modelData
                        required property int index
                        tip: modelData + " on the selection"
                        onClicked: App.setDynamic(index + 1)
                        contentItem: Label { text: modelData; font.italic: true; font.bold: true; font.family: "serif"; horizontalAlignment: Text.AlignHCenter; verticalAlignment: Text.AlignVCenter }
                    }
                }
            }

            SectionLabel { text: "Lines" }
            RowLayout {
                spacing: 2
                PanelButton {
                    objectName: "slurButton"
                    tip: "Slur over the selection, or from the selected note to the next (S)"
                    onClicked: App.addSlur()
                    contentItem: Label { text: "⌒"; font.pixelSize: 20; horizontalAlignment: Text.AlignHCenter; verticalAlignment: Text.AlignVCenter }
                }
                PanelButton {
                    tip: "Crescendo hairpin (H)"
                    onClicked: App.addHairpin(true)
                    contentItem: Label { text: "<"; font.pixelSize: 20; horizontalAlignment: Text.AlignHCenter; verticalAlignment: Text.AlignVCenter; transform: Scale { xScale: 1.6; origin.x: 17 } }
                }
                PanelButton {
                    tip: "Diminuendo hairpin (Shift+H)"
                    onClicked: App.addHairpin(false)
                    contentItem: Label { text: ">"; font.pixelSize: 20; horizontalAlignment: Text.AlignHCenter; verticalAlignment: Text.AlignVCenter; transform: Scale { xScale: 1.6; origin.x: 17 } }
                }
            }

            SectionLabel { text: "Selection" }
            GridLayout {
                columns: 2
                columnSpacing: 2
                rowSpacing: 2
                Repeater {
                    model: [ { t: "Copy", k: "Ctrl+C", f: () => App.copySelection(), e: () => App.hasRange },
                             { t: "Paste", k: "Ctrl+V", f: () => App.paste(), e: () => App.canPaste },
                             { t: "Repeat", k: "R", f: () => App.repeatSelection(), e: () => App.hasRange },
                             { t: "All", k: "Ctrl+A", f: () => App.selectAll(), e: () => true } ]
                    delegate: PanelButton {
                        required property var modelData
                        objectName: "sel" + modelData.t
                        implicitWidth: 52
                        enabled: modelData.e()
                        tip: modelData.t + " (" + modelData.k + "). Drag on empty paper to marquee-select; Shift+click extends."
                        onClicked: modelData.f()
                        contentItem: Label { text: modelData.t; font.pixelSize: 11; horizontalAlignment: Text.AlignHCenter; verticalAlignment: Text.AlignVCenter; opacity: parent.enabled ? 1 : 0.4 }
                    }
                }
            }

            SectionLabel { text: "Voice (V)" }
            RowLayout {
                spacing: 2
                Repeater {
                    model: 4
                    delegate: PanelButton {
                        required property int index
                        implicitWidth: 26
                        checked: App.voice === index + 1
                        tip: "Voice " + (index + 1)
                        onClicked: App.voice = index + 1
                        contentItem: Label { text: index + 1; horizontalAlignment: Text.AlignHCenter; verticalAlignment: Text.AlignVCenter; color: parent.checked ? "white" : "black" }
                    }
                }
            }
            Item { Layout.fillHeight: true }
        }
    }
}
