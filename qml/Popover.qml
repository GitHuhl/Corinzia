import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import Corinzia.Core

// Dorico-style popover: one text field that understands musical shorthand
// ("Eb", "6/8", "Allegro 132", ":|" …). Enter applies at the selection or
// caret, Esc cancels. Errors keep it open and say what was expected.
Rectangle {
    id: pop
    property string kind: ""
    property string where: ""
    signal closed()

    readonly property var info: ({
        key:        { title: "Key signature",  hint: "C  Eb  F#  ·  lower case or m = minor: f#, Bbm, a minor" },
        time:       { title: "Time signature", hint: "4/4  3/4  6/8  5/8  ·  C = common, C| = cut" },
        clef:       { title: "Clef",           hint: "treble  bass  alto  tenor  treble_8  bass_8" },
        barline:    { title: "Barline",        hint: "||  double  ·  |.  final  ·  |:  :|  :|:  repeats" },
        tempo:      { title: "Tempo",          hint: "Allegro  ·  Andante 84  ·  q=96  ·  72" },
        expression: { title: "Expression",     hint: "dolce, espressivo, sempre legato … (below the staff, italic)" },
        technique:  { title: "Playing technique", hint: "pizz., arco, con sord., sul pont. … (above the staff)" }
    })

    visible: false
    width: 380
    height: col.implicitHeight + 20
    radius: 6
    color: "#fdfdfb"
    border.color: "#2f6fb5"
    border.width: 2

    function open(k) {
        kind = k
        where = App.popoverPosition()
        error.text = ""
        field.text = ""
        visible = true
        field.forceActiveFocus()
    }
    function close() {
        visible = false
        closed()
    }

    ColumnLayout {
        id: col
        anchors.fill: parent
        anchors.margins: 10
        spacing: 4
        Label {
            text: (pop.info[pop.kind] ? pop.info[pop.kind].title : "") + "  ·  " + pop.where
            font.bold: true
        }
        TextField {
            id: field
            objectName: "popoverField"
            Layout.fillWidth: true
            placeholderText: pop.info[pop.kind] ? pop.info[pop.kind].hint : ""
            Keys.onReturnPressed: apply()
            Keys.onEnterPressed: apply()
            Keys.onEscapePressed: pop.close()
            function apply() {
                if (App.applyPopover(pop.kind, text)) pop.close()
                else error.text = App.status
            }
        }
        Label {
            id: error
            objectName: "popoverError"
            visible: text.length > 0
            color: "#a02020"
            font.pixelSize: 11
            Layout.fillWidth: true
            wrapMode: Text.Wrap
        }
        Label {
            text: "Enter to apply · Esc to cancel"
            font.pixelSize: 10
            opacity: 0.6
        }
    }
}
