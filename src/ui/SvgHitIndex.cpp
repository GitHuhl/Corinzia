#include "corinzia/ui/SvgHitIndex.hpp"

#include <QString>
#include <QSvgRenderer>
#include <QRegularExpression>
#include <QXmlStreamReader>

#include <cmath>

namespace corinzia::ui {

namespace {
std::uint64_t base36(QStringView s) {
    std::uint64_t v = 0;
    for (QChar c : s) {
        const char ch = char(c.toLatin1());
        int d = (ch >= '0' && ch <= '9') ? ch - '0' : (ch >= 'a' && ch <= 'z') ? ch - 'a' + 10 : -1;
        if (d < 0) return 0;
        v = v * 36 + std::uint64_t(d);
    }
    return v;
}
}  // namespace

void SvgHitIndex::addPage(int page, const QByteArray& svg) {
    QSvgRenderer renderer(svg);
    if (int(pageSizes_.size()) <= page) pageSizes_.resize(std::size_t(page) + 1);
    pageSizes_[std::size_t(page)] = renderer.viewBoxF().size();

    QXmlStreamReader xml(svg);
    while (!xml.atEnd()) {
        if (xml.readNext() != QXmlStreamReader::StartElement || xml.name() != QLatin1String("g")) continue;
        const auto attrs = xml.attributes();
        const QString cls = attrs.value(QLatin1String("class")).toString();
        if (cls.startsWith(QLatin1String("staff "))) {   // StaffSymbol: <g translate><line/></g> x5
            Staff st;
            st.page = page;
            for (auto c : QStringView(cls).split(u' ', Qt::SkipEmptyParts))
                if (c.startsWith(u"ln-")) st.lane = std::size_t(c.mid(3).toUInt());
            static const QRegularExpression tr(QStringLiteral("translate\\(\\s*([-0-9.]+)[ ,]+([-0-9.]+)\\s*\\)"));
            int depth = 1;
            qreal tx = 0, ty = 0;
            st.left = 1e9;
            st.right = -1e9;
            while (depth > 0 && !xml.atEnd()) {
                const auto t = xml.readNext();
                if (t == QXmlStreamReader::EndElement) { --depth; continue; }
                if (t != QXmlStreamReader::StartElement) continue;
                ++depth;
                const auto a = xml.attributes();
                if (xml.name() == QLatin1String("g")) {
                    const auto m = tr.match(a.value(QLatin1String("transform")).toString());
                    if (m.hasMatch()) { tx = m.captured(1).toDouble(); ty = m.captured(2).toDouble(); }
                } else if (xml.name() == QLatin1String("line")) {
                    const qreal y = ty + a.value(QLatin1String("y1")).toDouble();
                    st.lineYs.push_back(y);
                    st.left = std::min(st.left, tx + a.value(QLatin1String("x1")).toDouble());
                    st.right = std::max(st.right, tx + a.value(QLatin1String("x2")).toDouble());
                    if (a.hasAttribute(QLatin1String("stroke-width"))) st.thickness = a.value(QLatin1String("stroke-width")).toDouble();
                }
            }
            std::sort(st.lineYs.begin(), st.lineYs.end());
            if (st.lineYs.size() >= 2) staves_.push_back(std::move(st));
            continue;
        }
        const QString idStr = attrs.value(QLatin1String("id")).toString();
        if (!idStr.startsWith(QLatin1String("e-"))) continue;
        const ElementId id = ElementId::fromSvgId(idStr.toStdString());
        if (!id.valid() || byId_.count(id)) continue;

        Entry e;
        e.id = id;
        e.page = page;
        e.bounds = renderer.boundsOnElement(idStr);
        const auto classes = attrs.value(QLatin1String("class")).split(u' ', Qt::SkipEmptyParts);
        for (auto c : classes) {
            if (c == u"note") e.kind = Kind::Note;
            else if (c == u"head") e.kind = Kind::Head;
            else if (c == u"rest") e.kind = Kind::Rest;
            else if (c.startsWith(u"v-")) e.voice = base36(c.mid(2));
            else if (c.startsWith(u"t-")) e.track = base36(c.mid(2));
        }
        const std::size_t idx = entries_.size();
        entries_.push_back(e);
        byId_[id] = idx;

        const int x0 = int(std::floor(e.bounds.left() / kCell)), x1 = int(std::floor(e.bounds.right() / kCell));
        const int y0 = int(std::floor(e.bounds.top() / kCell)), y1 = int(std::floor(e.bounds.bottom() / kCell));
        for (int cx = x0; cx <= x1; ++cx)
            for (int cy = y0; cy <= y1; ++cy) grid_[cellKey(page, cx, cy)].push_back(idx);
    }
}

const SvgHitIndex::Entry* SvgHitIndex::find(ElementId id) const {
    auto it = byId_.find(id);
    return it == byId_.end() ? nullptr : &entries_[it->second];
}

const SvgHitIndex::Entry* SvgHitIndex::hitTest(int page, QPointF pt, qreal tol) const {
    const Entry* best = nullptr;
    qreal bestArea = 0;
    const int cx0 = int(std::floor((pt.x() - tol) / kCell)), cx1 = int(std::floor((pt.x() + tol) / kCell));
    const int cy0 = int(std::floor((pt.y() - tol) / kCell)), cy1 = int(std::floor((pt.y() + tol) / kCell));
    for (int cx = cx0; cx <= cx1; ++cx)
        for (int cy = cy0; cy <= cy1; ++cy) {
            auto it = grid_.find(cellKey(page, cx, cy));
            if (it == grid_.end()) continue;
            for (std::size_t i : it->second) {
                const Entry& e = entries_[i];
                if (!e.bounds.adjusted(-tol, -tol, tol, tol).contains(pt)) continue;
                const qreal area = e.bounds.width() * e.bounds.height();
                if (!best || area < bestArea) { best = &e; bestArea = area; }   // innermost wins (head over chord)
            }
        }
    return best;
}

}  // namespace corinzia::ui
