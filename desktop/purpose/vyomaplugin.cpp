// SPDX-License-Identifier: GPL-3.0-or-later
// "Send to phone" in KDE's Share menu: the shared files go to `vyoma-desktop --send-any`, which
// sends them to the connected phone (asking which when there are several) and says how it went.
// Only files on this computer: a link or a remote file can't be handed over as a path.

#include <KPluginFactory>
#include <Purpose/PluginBase>

#include <QDir>
#include <QFileInfo>
#include <QJsonArray>
#include <QProcess>
#include <QStandardPaths>
#include <QUrl>

namespace
{
// Where install.sh and the packages put it; also wherever PATH finds it.
QString vyomaDesktop()
{
    const QByteArray forced = qgetenv("VYOMA_DESKTOP_BIN");
    if (!forced.isEmpty())
        return QString::fromLocal8Bit(forced);
    const QString found = QStandardPaths::findExecutable(QStringLiteral("vyoma-desktop"));
    if (!found.isEmpty())
        return found;
    const QString local = QDir::homePath() + QStringLiteral("/.local/bin/vyoma-desktop");
    return QFileInfo(local).isExecutable() ? local : QString();
}

// Purpose runs plugins in a process of their own and drops a job's error on the way back, so the
// reason is said here too, as vyoma-desktop says its own.
void tell(const QString &title, const QString &body)
{
    QProcess::startDetached(QStringLiteral("notify-send"),
                            {QStringLiteral("--app-name=Vyoma"), QStringLiteral("--icon=vyoma"), QStringLiteral("--hint=string:desktop-entry:vyoma"), title, body});
}
}

class VyomaJob : public Purpose::Job
{
    Q_OBJECT
public:
    using Purpose::Job::Job;

    void fail(const QString &why)
    {
        tell(QStringLiteral("Couldn't send"), why);
        setError(KJob::UserDefinedError);
        setErrorText(why);
    }

    void start() override
    {
        QStringList args{QStringLiteral("--send-any")};
        const QJsonArray urls = data().value(QStringLiteral("urls")).toArray();
        for (const auto &u : urls) {
            const QUrl url(u.toString());
            if (url.isLocalFile())
                args << url.toLocalFile();
        }
        const QString exe = vyomaDesktop();
        if (args.size() == 1)
            fail(QStringLiteral("Only files on this computer can go to your phone. Save it here first, then share it again."));
        else if (exe.isEmpty() || !QProcess::startDetached(exe, args))
            fail(QStringLiteral("Vyoma isn't installed on this computer."));
        // vyoma-desktop says the rest in notifications: sent, which phone, or what's in the way.
        emitResult();
    }
};

class VyomaPlugin : public Purpose::PluginBase
{
    Q_OBJECT
public:
    VyomaPlugin(QObject *parent, const QVariantList &)
        : Purpose::PluginBase(parent)
    {
    }

    Purpose::Job *createJob() const override
    {
        return new VyomaJob(nullptr);
    }
};

K_PLUGIN_CLASS_WITH_JSON(VyomaPlugin, "vyomaplugin.json")

#include "vyomaplugin.moc"
