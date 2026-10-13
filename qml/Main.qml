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
    Shortcut { sequence: "Up"; onActivated: App.transposeSelection(1) }
    Shortcut { sequence: "Down"; onActivated: App.transposeSelection(-1) }
    Shortcut { sequence: "Ctrl+Up"; onActivated: App.transposeSelection(7) }
    Shortcut { sequence: "Ctrl+Down"; onActivated: App.transposeSelection(-7) }

    header: ToolBar {
        RowLayout {
            anchors.fill: parent
            spacing: 6
            ToolButton { text: App.playing ? "■ Stop" : "▶ Play"; onClicked: App.togglePlay() }
            ToolSeparator {}
            ToolButton { text: "Undo"; enabled: App.canUndo; onClicked: App.undo() }
            ToolButton { text: "Redo"; enabled: App.canRedo; onClicked: App.redo() }
            ToolSeparator {}
            ToolButton { text: "▲"; ToolTip.text: "Transpose selection up (↑)"; ToolTip.visible: hovered; onClicked: App.transposeSelection(1) }
            ToolButton { text: "▼"; ToolTip.text: "Transpose selection down (↓)"; ToolTip.visible: hovered; onClicked: App.transposeSelection(-1) }
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
            Label { text: App.selectionLabel.length ? App.selectionLabel : "Click a note to select it" }
            Item { Layout.fillWidth: true }
            Label { text: App.status; elide: Text.ElideLeft; Layout.maximumWidth: 600 }
        }
    }

    Component.onCompleted: App.attachCanvas(score)
}
