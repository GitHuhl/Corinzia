import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import Corinzia.Core

// The DAW side: arrange (tracks × time), mixer strips and the plugin browser.
// Shares the project, selection and transport with the Score tab.
Item {
    id: mix
    property alias arrange: arrangeView

    SplitView {
        anchors.fill: parent
        orientation: Qt.Vertical

        // ---------------------------------------------------------- arrange --
        Item {
            SplitView.fillHeight: true
            SplitView.minimumHeight: 160

            Rectangle {   // track headers
                id: headers
                width: 190
                anchors.top: parent.top
                anchors.bottom: parent.bottom
                color: "#e9ebef"
                Rectangle { anchors.right: parent.right; width: 1; height: parent.height; color: "#c5c9d1" }
                Label { x: 8; y: 4; text: "Tracks"; font.bold: true; opacity: 0.7 }
                Column {
                    y: ruler.height
                    width: parent.width
                    Repeater {
                        model: App.tracks
                        delegate: Item {
                            required property var modelData
                            width: headers.width - 1
                            height: arrangeView.laneHeight
                            Rectangle { width: 5; height: parent.height; color: modelData.color }
                            Column {
                                x: 12; y: 6
                                spacing: 2
                                Label { text: modelData.name; font.bold: true }
                                Label { text: modelData.plugin + " · ch " + modelData.channel; opacity: 0.65; font.pixelSize: 11 }
                                Label { text: modelData.gainDb.toFixed(1) + " dB"; opacity: 0.65; font.pixelSize: 11 }
                            }
                            Rectangle { anchors.bottom: parent.bottom; width: parent.width; height: 1; color: "#d2d6de" }
                        }
                    }
                }
            }

            Item {
                anchors.left: headers.right
                anchors.right: parent.right
                anchors.top: parent.top
                anchors.bottom: parent.bottom

                Rectangle {   // bar ruler; click to locate
                    id: ruler
                    width: parent.width
                    height: 24
                    color: "#dfe2e8"
                    clip: true
                    Repeater {
                        model: App.arrangeBars
                        delegate: Item {
                            required property var modelData
                            x: modelData.quarter * arrangeView.pixelsPerQuarter - arrangeView.scrollX
                            height: ruler.height
                            Rectangle { width: 1; height: 8; anchors.bottom: parent.bottom; color: "#7a8090" }
                            Label { x: 3; y: 2; text: modelData.bar; font.pixelSize: 11 }
                        }
                    }
                    MouseArea {
                        anchors.fill: parent
                        onClicked: (m) => App.locate((m.x + arrangeView.scrollX) / arrangeView.pixelsPerQuarter)
                    }
                }

                ArrangeView {
                    id: arrangeView
                    objectName: "arrange"
                    anchors.top: ruler.bottom
                    anchors.left: parent.left
                    anchors.right: parent.right
                    anchors.bottom: parent.bottom
                    pixelsPerQuarter: arrangeZoom.value
                    focus: true
                    Keys.onPressed: (event) => {
                        if (event.key === Qt.Key_Delete || event.key === Qt.Key_Backspace) App.deleteSelection()
                        else if (event.key === Qt.Key_Up) App.transposeSelection(event.modifiers & Qt.ControlModifier ? 7 : 1)
                        else if (event.key === Qt.Key_Down) App.transposeSelection(event.modifiers & Qt.ControlModifier ? -7 : -1)
                        else return
                        event.accepted = true
                    }
                }

                Row {   // arrange zoom, bottom-right
                    anchors.right: parent.right
                    anchors.bottom: parent.bottom
                    anchors.margins: 6
                    spacing: 6
                    Label { text: "Zoom"; anchors.verticalCenter: parent.verticalCenter; opacity: 0.7 }
                    Slider { id: arrangeZoom; from: 8; to: 160; value: 40; width: 140; focusPolicy: Qt.NoFocus }
                }
            }
        }

        // ----------------------------------------------- mixer + plugins --
        RowLayout {
            SplitView.preferredHeight: 250
            SplitView.minimumHeight: 180
            spacing: 0

            ScrollView {
                Layout.fillWidth: true
                Layout.fillHeight: true
                contentHeight: availableHeight
                Row {
                    padding: 8
                    spacing: 8
                    height: parent.height
                    Repeater {
                        model: App.tracks
                        delegate: Frame {
                            id: strip
                            required property var modelData
                            required property int index
                            objectName: "strip" + index
                            width: 150
                            height: parent.height - 16
                            padding: 6
                            background: Rectangle { color: "#f6f7f9"; border.color: "#cfd3da"; radius: 4
                                Rectangle { width: parent.width; height: 4; color: strip.modelData.color; radius: 2 } }
                            ColumnLayout {
                                anchors.fill: parent
                                Label { text: strip.modelData.name; font.bold: true; Layout.topMargin: 4; elide: Text.ElideRight; Layout.fillWidth: true }
                                Label { text: strip.modelData.plugin; opacity: 0.65; font.pixelSize: 11; elide: Text.ElideRight; Layout.fillWidth: true }
                                RowLayout {
                                    Layout.fillHeight: true
                                    Slider {
                                        objectName: "fader"
                                        orientation: Qt.Vertical
                                        from: -60; to: 6
                                        value: strip.modelData.gainDb
                                        focusPolicy: Qt.NoFocus
                                        Layout.fillHeight: true
                                        onMoved: App.setTrackGain(strip.index, value)
                                    }
                                    Label { text: strip.modelData.gainDb.toFixed(1) + "\ndB"; horizontalAlignment: Text.AlignHCenter }
                                }
                                ComboBox {
                                    Layout.fillWidth: true
                                    model: App.plugins
                                    textRole: "name"
                                    focusPolicy: Qt.NoFocus
                                    displayText: App.plugins.length ? "Instrument…" : "No plugins"
                                    enabled: App.plugins.length > 0
                                    onActivated: (i) => App.assignPlugin(strip.index, i)
                                }
                            }
                        }
                    }
                }
            }

            Rectangle { width: 1; Layout.fillHeight: true; color: "#c5c9d1" }

            ColumnLayout {   // plugin browser
                Layout.preferredWidth: 280
                Layout.fillHeight: true
                Layout.margins: 8
                RowLayout {
                    Label { text: "CLAP plugins"; font.bold: true; Layout.fillWidth: true }
                    Button { text: "Rescan"; focusPolicy: Qt.NoFocus; onClicked: App.rescanPlugins() }
                }
                ListView {
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    clip: true
                    model: App.plugins
                    delegate: ItemDelegate {
                        required property var modelData
                        width: ListView.view.width
                        text: modelData.name + "  ·  " + modelData.vendor
                        ToolTip.text: modelData.features
                        ToolTip.visible: hovered && modelData.features.length > 0
                    }
                    Label {
                        anchors.centerIn: parent
                        visible: parent.count === 0
                        width: parent.width - 16
                        wrapMode: Text.Wrap
                        horizontalAlignment: Text.AlignHCenter
                        opacity: 0.6
                        text: "No CLAP plugins found in CLAP_PATH, ~/.clap, your Nix profiles or /usr/lib/clap."
                    }
                }
            }
        }
    }

    Component.onCompleted: App.attachArrange(arrangeView)
}
