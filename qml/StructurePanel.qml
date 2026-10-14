import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import Corinzia.Core

// Right-hand panel: score structure and text. Each button opens the same
// popover as its shortcut, or acts directly (rehearsal mark, barlines, bars).
Pane {
    id: panel
    padding: 6
    implicitWidth: 150
    signal openPopover(string kind)

    component PanelButton: Button {
        id: b
        property string tip: ""
        Layout.fillWidth: true
        implicitHeight: 28
        focusPolicy: Qt.NoFocus          // typing must keep going to the score
        font.pixelSize: 12
        ToolTip.text: tip
        ToolTip.visible: hovered && tip.length > 0
        ToolTip.delay: 400
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
            spacing: 3

            SectionLabel { text: "Structure" }
            PanelButton { objectName: "keyButton"; text: "Key…"; tip: "Key signature (Shift+K)"; onClicked: panel.openPopover("key") }
            PanelButton { objectName: "timeButton"; text: "Time…"; tip: "Time signature (Shift+M)"; onClicked: panel.openPopover("time") }
            PanelButton { text: "Clef…"; tip: "Clef change on this staff (Shift+C outside note input)"; onClicked: panel.openPopover("clef") }
            PanelButton { text: "Tempo…"; tip: "Tempo marking (Shift+T)"; onClicked: panel.openPopover("tempo") }

            SectionLabel { text: "Barlines" }
            GridLayout {
                columns: 3
                columnSpacing: 2
                rowSpacing: 2
                Layout.fillWidth: true
                Repeater {
                    model: [ { t: "||", n: "Double barline" }, { t: "|.", n: "Final barline" }, { t: "|:", n: "Start repeat" },
                             { t: ":|", n: "End repeat" }, { t: ":|:", n: "End and start repeat" } ]
                    delegate: PanelButton {
                        required property var modelData
                        text: modelData.t
                        tip: modelData.n + " at the nearest bar line (Shift+B for the popover)"
                        onClicked: App.applyPopover("barline", modelData.t)
                    }
                }
            }
            PanelButton {
                objectName: "endingsButton"
                text: "1st / 2nd ending"
                tip: "Selected bars become the 1st ending (with an end repeat); the same number of bars after them the 2nd"
                onClicked: App.addFirstSecondEndings()
            }
            PanelButton {
                text: "Ending…"
                tip: "Any ending over the selected bars: 1, 2, \"1, 2\", 1-3 (Shift+R outside note input)"
                onClicked: panel.openPopover("ending")
            }
            PanelButton {
                objectName: "rehearsalButton"
                text: "Rehearsal mark"
                tip: "Add or remove a lettered mark at this bar (Shift+A outside note input)"
                onClicked: App.applyPopover("rehearsal", "")
            }

            SectionLabel { text: "Text" }
            PanelButton { text: "Expression…"; tip: "Italic text below the staff, e.g. dolce (Shift+X)"; onClicked: panel.openPopover("expression") }
            PanelButton { text: "Technique…"; tip: "Text above the staff, e.g. pizz. (Shift+P)"; onClicked: panel.openPopover("technique") }

            SectionLabel { text: "Bars" }
            RowLayout {
                spacing: 2
                Layout.fillWidth: true
                property int count: 1
                id: barCount
                PanelButton { text: "−"; Layout.preferredWidth: 30; Layout.fillWidth: false; enabled: barCount.count > 1; onClicked: barCount.count-- }
                Label {
                    text: barCount.count + (barCount.count === 1 ? " bar" : " bars")
                    horizontalAlignment: Text.AlignHCenter
                    Layout.fillWidth: true
                    font.pixelSize: 12
                }
                PanelButton { text: "+"; Layout.preferredWidth: 30; Layout.fillWidth: false; enabled: barCount.count < 64; onClicked: barCount.count++ }
            }
            PanelButton {
                objectName: "insertBarsButton"
                text: "Insert before"
                tip: "Insert empty bars before the bar at the selection or caret (Ctrl+B)"
                onClicked: App.insertBars(barCount.count)
            }
            PanelButton {
                objectName: "deleteBarsButton"
                text: "Delete bars"
                tip: "Delete the selected bars, or the bar at the caret (Ctrl+Shift+Backspace)"
                onClicked: App.deleteBars()
            }
            Label {
                Layout.topMargin: 8
                Layout.fillWidth: true
                wrapMode: Text.Wrap
                font.pixelSize: 10
                opacity: 0.6
                text: "Click a barline, mark, clef or text in the score and press Delete to remove it."
            }
        }
    }
}
