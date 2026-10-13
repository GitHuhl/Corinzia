import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import Corinzia.Core

ApplicationWindow {
    id: window
    width: 1400
    height: 900
    visible: true
    title: App.title + " — Corinzia"

    Shortcut { sequences: [StandardKey.Undo]; onActivated: App.undo() }
    Shortcut { sequences: [StandardKey.Redo]; onActivated: App.redo() }
    Shortcut { sequence: "Space"; onActivated: App.togglePlay() }

    // Score keyboard (Dorico-like). Lives on the canvas so text fields elsewhere
    // keep their keys; panel buttons never take focus.
    function handleScoreKey(event) {
        const k = event.key
        const shift = event.modifiers & Qt.ShiftModifier
        const ctrl = event.modifiers & Qt.ControlModifier
        const alt = event.modifiers & Qt.AltModifier
        const letters = { [Qt.Key_C]: 0, [Qt.Key_D]: 1, [Qt.Key_E]: 2, [Qt.Key_F]: 3, [Qt.Key_G]: 4, [Qt.Key_A]: 5, [Qt.Key_B]: 6 }
        if (ctrl && k !== Qt.Key_Up && k !== Qt.Key_Down) return
        if (k === Qt.Key_N) App.inputMode = !App.inputMode
        else if (k === Qt.Key_Escape) App.inputMode = false
        else if (App.inputMode && letters[k] !== undefined) App.enterNote(letters[k], shift)
        else if (k >= Qt.Key_1 && k <= Qt.Key_7) App.duration = 7 - (k - Qt.Key_0)
        else if (k === Qt.Key_Period) App.dots = (App.dots + 1) % 3
        else if (k === Qt.Key_Semicolon) App.triplet = !App.triplet
        else if (App.inputMode && (k === Qt.Key_R || k === Qt.Key_Comma)) App.enterRest()
        else if (k === Qt.Key_Q) App.chordMode = !App.chordMode
        else if (k === Qt.Key_T) App.toggleTie()
        else if (k === Qt.Key_V) App.cycleVoice()
        else if (k === Qt.Key_Minus) App.accidental = App.accidental === -1 ? 9 : -1
        else if (k === Qt.Key_Equal || k === Qt.Key_Plus) App.accidental = App.accidental === 1 ? 9 : 1
        else if (k === Qt.Key_0) App.accidental = App.accidental === 0 ? 9 : 0
        else if (k === Qt.Key_BracketLeft) App.toggleArticulation(1)
        else if (k === Qt.Key_Apostrophe) App.toggleArticulation(2)
        else if (k === Qt.Key_BracketRight) App.toggleArticulation(4)
        else if (k === Qt.Key_Backslash) App.toggleArticulation(8)
        else if (k === Qt.Key_Left) App.moveCaret(-1)
        else if (k === Qt.Key_Right) App.moveCaret(1)
        else if (k === Qt.Key_Up && alt) App.moveCaretStaff(-1)
        else if (k === Qt.Key_Down && alt) App.moveCaretStaff(1)
        else if (k === Qt.Key_Up) App.transposeSelection(ctrl ? 7 : 1)
        else if (k === Qt.Key_Down) App.transposeSelection(ctrl ? -7 : -1)
        else if (k === Qt.Key_Backspace) App.inputMode ? App.backspace() : App.deleteSelection()
        else if (k === Qt.Key_Delete) App.deleteSelection()
        else return
        event.accepted = true
    }

    header: ToolBar {
        RowLayout {
            anchors.fill: parent
            spacing: 6
            ToolButton { text: App.playing ? "■ Stop" : "▶ Play"; onClicked: App.togglePlay() }
            ToolSeparator {}
            ToolButton { text: "Undo"; enabled: App.canUndo; onClicked: App.undo() }
            ToolButton { text: "Redo"; enabled: App.canRedo; onClicked: App.redo() }
            ToolSeparator {}
            ToolButton { text: "▲"; focusPolicy: Qt.NoFocus; ToolTip.text: "Transpose selection up (↑, Ctrl+↑ octave)"; ToolTip.visible: hovered; onClicked: App.transposeSelection(1) }
            ToolButton { text: "▼"; focusPolicy: Qt.NoFocus; ToolTip.text: "Transpose selection down (↓, Ctrl+↓ octave)"; ToolTip.visible: hovered; onClicked: App.transposeSelection(-1) }
            ToolSeparator {}
            ToolButton {
                text: App.inputMode ? "✎ Input ON" : "✎ Input"
                focusPolicy: Qt.NoFocus
                checkable: true
                checked: App.inputMode
                onToggled: App.inputMode = checked
                ToolTip.text: "Note input (N). Letters A–G enter notes, 1–7 durations, R rest, Esc exits"
                ToolTip.visible: hovered
            }
            ToolSeparator {}
            Label { text: "Zoom" }
            Slider {
                id: zoomSlider
                from: 0.5; to: 4; value: 1.2
                Layout.preferredWidth: 160
                onMoved: score.zoom = value
            }
            Item { Layout.fillWidth: true }
        }
    }

    SplitView {
        anchors.fill: parent

        NotesPanel {
            objectName: "notesPanel"
            SplitView.preferredWidth: 128
            SplitView.minimumWidth: 110
        }

        Rectangle {
            SplitView.fillWidth: true
            color: "#d9d6cf"
            clip: true
            ScoreCanvas {
                id: score
                objectName: "score"
                anchors.fill: parent
                zoom: zoomSlider.value
                pan: Qt.point(24, 24)
                onZoomChanged: zoomSlider.value = zoom
                focus: true
                Keys.onPressed: (event) => window.handleScoreKey(event)
            }
        }

        Pane {
            SplitView.preferredWidth: 340
            ScrollView {
                anchors.fill: parent
                ColumnLayout {
                    width: 300
                    spacing: 10
                    Label { text: "Mixer"; font.bold: true; font.pixelSize: 16 }
                    Repeater {
                        model: App.tracks
                        delegate: Frame {
                            required property var modelData
                            required property int index
                            Layout.fillWidth: true
                            ColumnLayout {
                                anchors.fill: parent
                                Label { text: modelData.name + "  ·  " + modelData.plugin; elide: Text.ElideRight; Layout.fillWidth: true }
                                RowLayout {
                                    Slider {
                                        from: -60; to: 6; value: modelData.gainDb
                                        Layout.fillWidth: true
                                        onMoved: App.setTrackGain(index, value)
                                    }
                                    Label { text: modelData.gainDb.toFixed(1) + " dB"; Layout.preferredWidth: 60 }
                                }
                                ComboBox {
                                    Layout.fillWidth: true
                                    model: App.plugins
                                    textRole: "name"
                                    displayText: App.plugins.length ? "Assign CLAP instrument…" : "No CLAP plugins found"
                                    enabled: App.plugins.length > 0
                                    onActivated: (i) => App.assignPlugin(index, i)
                                }
                            }
                        }
                    }
                    Button { text: "Rescan plugins"; onClicked: App.rescanPlugins() }
                    Label {
                        text: App.plugins.length + " CLAP plugin(s) in CLAP_PATH, ~/.clap, Nix profiles, /usr/lib/clap"
                        wrapMode: Text.Wrap
                        Layout.fillWidth: true
                        opacity: 0.7
                    }
                }
            }
        }
    }

    footer: ToolBar {
        RowLayout {
            anchors.fill: parent
            anchors.leftMargin: 8
            anchors.rightMargin: 8
            Label {
                text: App.inputMode ? "✎ " + App.caretLabel
                                    : (App.selectionLabel.length ? App.selectionLabel : "Click a note to select it · N to write notes")
                color: App.inputMode ? "#0a6a3a" : palette.text
            }
            Item { Layout.fillWidth: true }
            Label { text: App.status; elide: Text.ElideLeft; Layout.maximumWidth: 600 }
        }
    }

    Component.onCompleted: App.attachCanvas(score)
}
