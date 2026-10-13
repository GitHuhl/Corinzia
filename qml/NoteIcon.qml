import QtQuick

// Duration icon drawn with Canvas (no music font dependency).
// value: 0=whole 1=half 2=quarter 3=8th 4=16th 5=32nd 6=64th
Canvas {
    id: icon
    property int value: 2
    property int dots: 0
    property bool rest: false
    property color color: "black"
    implicitWidth: 28
    implicitHeight: 34
    onValueChanged: requestPaint()
    onDotsChanged: requestPaint()
    onRestChanged: requestPaint()
    onColorChanged: requestPaint()

    onPaint: {
        const ctx = getContext("2d")
        ctx.reset()
        ctx.fillStyle = color
        ctx.strokeStyle = color
        ctx.lineWidth = 1.6
        const flags = Math.max(0, value - 2)
        if (rest) {
            const cx = width * 0.42
            if (value === 0) ctx.fillRect(cx - 6, height * 0.40, 12, 4)          // hangs from a line
            else if (value === 1) ctx.fillRect(cx - 6, height * 0.50, 12, 4)     // sits on a line
            else if (value === 2) {                                              // crotchet rest zigzag
                ctx.beginPath()
                ctx.moveTo(cx - 2, height * 0.18); ctx.lineTo(cx + 4, height * 0.36)
                ctx.lineTo(cx - 3, height * 0.52); ctx.lineTo(cx + 4, height * 0.68)
                ctx.lineTo(cx - 2, height * 0.74); ctx.lineTo(cx + 1, height * 0.86)
                ctx.lineWidth = 2.6
                ctx.stroke()
            } else {                                                             // 8th+ : stem with blobs
                ctx.beginPath()
                ctx.moveTo(cx + 5, height * 0.25); ctx.lineTo(cx - 1, height * 0.85)
                ctx.stroke()
                for (let i = 0; i < flags; ++i) {
                    ctx.beginPath()
                    ctx.arc(cx - 2 - i * 1.5, height * (0.28 + i * 0.16), 2.6, 0, 2 * Math.PI)
                    ctx.fill()
                }
            }
        } else {
            const hx = width * 0.36, hy = height * 0.76
            ctx.save()
            ctx.translate(hx, hy)
            ctx.rotate(-0.35)
            ctx.beginPath()
            ctx.ellipse(-6, -4.2, 12, 8.4)
            if (value >= 2) ctx.fill()
            else { ctx.lineWidth = 2; ctx.stroke() }
            ctx.restore()
            if (value >= 1) {                                                    // stem
                ctx.beginPath()
                ctx.moveTo(hx + 5.4, hy - 1.5)
                ctx.lineTo(hx + 5.4, height * 0.10)
                ctx.stroke()
            }
            for (let i = 0; i < flags; ++i) {                                    // flags
                ctx.beginPath()
                const y = height * 0.10 + i * 5
                ctx.moveTo(hx + 5.4, y)
                ctx.quadraticCurveTo(hx + 13, y + 6, hx + 11, y + 12)
                ctx.stroke()
            }
        }
        for (let d = 0; d < dots; ++d) {
            ctx.beginPath()
            ctx.arc(width * 0.74 + d * 5, height * 0.72, 1.8, 0, 2 * Math.PI)
            ctx.fill()
        }
    }
}
