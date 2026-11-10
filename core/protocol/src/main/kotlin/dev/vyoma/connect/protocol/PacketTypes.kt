package dev.vyoma.connect.protocol

/** Wire packet types. Stock KDE Connect types first; `vyoma.*` are our extensions. */
object PacketTypes {
    const val IDENTITY = "kdeconnect.identity"
    const val PAIR = "kdeconnect.pair"

    const val PING = "kdeconnect.ping"
    const val BATTERY = "kdeconnect.battery"
    const val BATTERY_REQUEST = "kdeconnect.battery.request"
    const val CLIPBOARD = "kdeconnect.clipboard"
    const val CLIPBOARD_CONNECT = "kdeconnect.clipboard.connect"
    const val CLIPBOARD_FILE = "kdeconnect.clipboard.file"
    const val FIND_MY_PHONE_REQUEST = "kdeconnect.findmyphone.request"
    const val SHARE_REQUEST = "kdeconnect.share.request"
    const val SHARE_REQUEST_UPDATE = "kdeconnect.share.request.update"
    const val NOTIFICATION = "kdeconnect.notification"
    const val NOTIFICATION_REQUEST = "kdeconnect.notification.request"
    const val NOTIFICATION_REPLY = "kdeconnect.notification.reply"
    const val NOTIFICATION_ACTION = "kdeconnect.notification.action"
    const val MPRIS = "kdeconnect.mpris"
    const val MPRIS_REQUEST = "kdeconnect.mpris.request"
    const val MOUSEPAD_REQUEST = "kdeconnect.mousepad.request"
    const val MOUSEPAD_ECHO = "kdeconnect.mousepad.echo"
    const val MOUSEPAD_KEYBOARDSTATE = "kdeconnect.mousepad.keyboardstate"
    const val PRESENTER = "kdeconnect.presenter"
    const val RUNCOMMAND = "kdeconnect.runcommand"
    const val RUNCOMMAND_REQUEST = "kdeconnect.runcommand.request"
    const val RUNCOMMAND_OUTPUT = "kdeconnect.runcommand.output"
    const val LOCK = "kdeconnect.lock"
    const val LOCK_REQUEST = "kdeconnect.lock.request"
    const val SYSTEMVOLUME = "kdeconnect.systemvolume"
    const val SYSTEMVOLUME_REQUEST = "kdeconnect.systemvolume.request"
    const val TELEPHONY = "kdeconnect.telephony"
    const val TELEPHONY_REQUEST_MUTE = "kdeconnect.telephony.request_mute"
    const val SMS_MESSAGES = "kdeconnect.sms.messages"
    const val SMS_REQUEST = "kdeconnect.sms.request"
    const val SMS_REQUEST_CONVERSATIONS = "kdeconnect.sms.request_conversations"
    const val SMS_REQUEST_CONVERSATION = "kdeconnect.sms.request_conversation"
    const val SMS_REQUEST_ATTACHMENT = "kdeconnect.sms.request_attachment"
    const val SMS_ATTACHMENT_FILE = "kdeconnect.sms.attachment_file"
    const val CONTACTS_REQUEST_ALL_UIDS_TIMESTAMPS = "kdeconnect.contacts.request_all_uids_timestamps"
    const val CONTACTS_REQUEST_VCARDS_BY_UIDS = "kdeconnect.contacts.request_vcards_by_uid"
    const val CONTACTS_RESPONSE_UIDS_TIMESTAMPS = "kdeconnect.contacts.response_uids_timestamps"
    const val CONTACTS_RESPONSE_VCARDS = "kdeconnect.contacts.response_vcards"

    /** PC → phone: `{action: answer|decline|hangup}` for the current call. */
    const val CALL_REQUEST = "vyoma.call.request"

    /** Phone → PC: `{action, error?}`, how that went. */
    const val CALL_DONE = "vyoma.call.done"

    /** PC → phone: someone opened Messages on the PC; ask the owner (fingerprint) before texts go there. */
    const val TEXTS_UNLOCK = "vyoma.texts.unlock"

    /** PC → phone: the PC locked, or Messages sat unused: no more texts until asked again. */
    const val TEXTS_LOCK = "vyoma.texts.lock"

    /** Phone → PC: `{ok}`, the owner's answer to [TEXTS_UNLOCK]. */
    const val TEXTS_UNLOCKED = "vyoma.texts.unlocked"

    /** Phone → PC: a request for texts was refused, as Messages is locked for this PC. */
    const val TEXTS_LOCKED = "vyoma.texts.locked"
    const val CONNECTIVITY_REPORT = "kdeconnect.connectivity_report"
    const val CONNECTIVITY_REPORT_REQUEST = "kdeconnect.connectivity_report.request"
    const val SFTP = "kdeconnect.sftp"
    const val SFTP_REQUEST = "kdeconnect.sftp.request"
}
