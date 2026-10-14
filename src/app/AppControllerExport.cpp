// Export: PDF through LilyPond (the same engraving as the score view, in
// Page layout), MIDI and MusicXML through the Rust export crate.

#include "corinzia/app/AppController.hpp"
#include "corinzia/engrave/LilyPondEmitter.hpp"
#include "corinzia/export/Export.hpp"

#include <QDir>
#include <QFile>
#include <QFileInfo>
#include <QProcess>
#include <QTemporaryDir>
#include <QUrl>

namespace corinzia::app {

namespace {
// File dialogs hand over URLs; tests and scripts pass plain paths.
QString localPath(const QString& pathOrUrl) {
    const QUrl url(pathOrUrl);
    return url.isLocalFile() ? url.toLocalFile() : pathOrUrl;
}
QString withSuffix(QString path, const QString& suffix, const QStringList& accepted) {
    const QString ext = QFileInfo(path).suffix().toLower();
    if (!accepted.contains(ext)) path += QLatin1Char('.') + suffix;
    return path;
}
}  // namespace

bool AppController::exportMidi(const QString& target) {
    const model::Movement* m = project_.movement(displayedMovement());
    if (!m) return false;
    const QString path = withSuffix(localPath(target), QStringLiteral("mid"), {QStringLiteral("mid"), QStringLiteral("midi")});
    const std::string err = exporter::writeMidi(project_, *m, path.toStdString());
    setStatus(err.empty() ? QStringLiteral("Exported MIDI: %1").arg(path) : QStringLiteral("MIDI export failed: %1").arg(QString::fromStdString(err)));
    return err.empty();
}

bool AppController::exportMusicXml(const QString& target) {
    const model::Movement* m = project_.movement(displayedMovement());
    if (!m) return false;
    const QString path = withSuffix(localPath(target), QStringLiteral("musicxml"), {QStringLiteral("musicxml"), QStringLiteral("xml")});
    const std::string err = exporter::writeMusicXml(project_, *m, path.toStdString());
    setStatus(err.empty() ? QStringLiteral("Exported MusicXML: %1").arg(path)
                          : QStringLiteral("MusicXML export failed: %1").arg(QString::fromStdString(err)));
    return err.empty();
}

// LilyPond runs in the background (a few seconds); `exportFinished` reports.
bool AppController::exportPdf(const QString& target) {
    const model::Movement* m = project_.movement(displayedMovement());
    if (!m) return false;
    const QString path = withSuffix(localPath(target), QStringLiteral("pdf"), {QStringLiteral("pdf")});
    engrave::EngraveContext ctx = engraveContext();
    ctx.panorama = false;   // always the printed layout
    auto dir = std::make_shared<QTemporaryDir>();
    if (!dir->isValid()) return false;
    const QString base = dir->filePath(QStringLiteral("score"));
    {
        QFile f(base + QStringLiteral(".ly"));
        if (!f.open(QIODevice::WriteOnly)) return false;
        f.write(QByteArray::fromStdString(engrave::emitLilyPond(*m, ctx)));
    }
    auto* proc = new QProcess(this);
    proc->setProcessChannelMode(QProcess::MergedChannels);
    connect(proc, &QProcess::finished, this, [this, proc, dir, base, path](int code, QProcess::ExitStatus st) {
        proc->deleteLater();
        const QString pdf = base + QStringLiteral(".pdf");
        bool ok = st == QProcess::NormalExit && code == 0 && QFile::exists(pdf);
        if (ok) {
            QFile::remove(path);
            ok = QFile::copy(pdf, path);
        }
        setStatus(ok ? QStringLiteral("Exported PDF: %1").arg(path)
                     : QStringLiteral("PDF export failed: %1").arg(QString::fromUtf8(proc->readAll()).right(300)));
        emit exportFinished(ok, path);
    });
    setStatus(QStringLiteral("Exporting PDF…"));
    proc->start(opt_.lilypond, {QStringLiteral("--pdf"), QStringLiteral("-s"), QStringLiteral("-o"), base, base + QStringLiteral(".ly")});
    return true;
}

}  // namespace corinzia::app
