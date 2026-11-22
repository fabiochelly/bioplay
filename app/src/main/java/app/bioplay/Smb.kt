package app.bioplay

import android.content.Context
import android.net.ConnectivityManager
import android.net.Uri
import android.os.Handler
import android.os.HandlerThread
import android.os.ParcelFileDescriptor
import android.os.ProxyFileDescriptorCallback
import android.os.storage.StorageManager
import android.system.ErrnoException
import android.system.OsConstants
import com.hierynomus.msdtyp.AccessMask
import com.hierynomus.mssmb2.SMB2CreateDisposition
import com.hierynomus.mssmb2.SMB2ShareAccess
import com.hierynomus.security.bc.BCSecurityProvider
import com.hierynomus.smbj.SMBClient
import com.hierynomus.smbj.SmbConfig
import com.hierynomus.smbj.auth.AuthenticationContext
import com.hierynomus.smbj.session.Session as SmbSession
import com.hierynomus.smbj.share.DiskShare
import com.rapid7.client.dcerpc.mssrvs.ServerService
import com.rapid7.client.dcerpc.transport.SMBTransportFactories
import org.json.JSONObject
import java.util.EnumSet
import java.util.concurrent.TimeUnit

/**
 * Partages réseau SMB 2/3 (disque d'une box, NAS). Adresses `smb://hôte/partage/chemin` ;
 * identifiants gardés à part, dans les préférences privées de l'app, par hôte.
 *
 * Un fichier est remis à qui le lit (mpv, l'index des sous-titres, les miniatures) comme un
 * descripteur ordinaire (`openProxyFileDescriptor`) : tout le reste de l'app reste inchangé.
 */
object Smb {
    class Entry(val name: String, val dir: Boolean, val size: Long, val modified: Long)
    class Login(val user: String, val password: String, val domain: String = "")

    private val client by lazy {
        SMBClient(
            SmbConfig.builder()
                // Android n'a pas MD4 (authentification NTLM) : Bouncy Castle le fournit.
                .withSecurityProvider(BCSecurityProvider())
                .withMultiProtocolNegotiate(true)
                .withDfsEnabled(false)
                .withTimeout(15, TimeUnit.SECONDS)
                .withSoTimeout(30, TimeUnit.SECONDS)
                .build()
        )
    }
    private val sessions = HashMap<String, SmbSession>()
    private val shares = HashMap<String, DiskShare>()

    fun isSmb(u: Uri) = u.scheme == "smb"
    fun isSmb(u: String) = u.startsWith("smb://")

    fun uri(host: String, share: String = "", path: String = ""): Uri = Uri.Builder().scheme("smb").authority(host).apply {
        if (share.isNotEmpty()) appendPath(share)
        path.split('/').filter { it.isNotEmpty() }.forEach { appendPath(it) }
    }.build()

    private fun shareOf(u: Uri) = u.pathSegments.firstOrNull().orEmpty()
    /** Chemin dans le partage, au format SMB (`a\b\c`). */
    private fun pathOf(u: Uri) = u.pathSegments.drop(1).joinToString("\\")

    // --- Identifiants -----------------------------------------------------------------------

    private fun prefs(c: Context) = c.getSharedPreferences("bioplay", Context.MODE_PRIVATE)

    fun login(c: Context, host: String): Login? = prefs(c).getString("smb:$host", null)?.let {
        runCatching { JSONObject(it).let { j -> Login(j.optString("u"), j.optString("p"), j.optString("d")) } }.getOrNull()
    }

    fun saveLogin(c: Context, host: String, l: Login) {
        prefs(c).edit().putString("smb:$host", JSONObject().put("u", l.user).put("p", l.password).put("d", l.domain).toString()).apply()
    }

    /** Adresse de la box (passerelle du réseau local), proposée par défaut. */
    fun gateway(c: Context): String? = runCatching {
        val cm = c.getSystemService(ConnectivityManager::class.java)
        cm.getLinkProperties(cm.activeNetwork)?.routes
            ?.firstOrNull { it.isDefaultRoute && it.gateway?.hostAddress?.contains('.') == true }
            ?.gateway?.hostAddress
    }.getOrNull()

    // --- Connexion ----------------------------------------------------------------------------

    @Synchronized
    private fun session(c: Context, host: String, login: Login? = login(c, host)): SmbSession {
        sessions[host]?.takeIf { it.connection.isConnected }?.let { return it }
        shares.keys.filter { it.startsWith("$host/") }.forEach { shares.remove(it) }
        // « hôte:port » accepté (serveur sur un port non standard).
        val port = host.substringAfterLast(':', "").toIntOrNull()
        val conn = if (port != null) client.connect(host.substringBeforeLast(':'), port) else client.connect(host)
        val auth = when {
            login == null || login.user.isBlank() -> AuthenticationContext.guest()
            else -> AuthenticationContext(login.user, login.password.toCharArray(), login.domain)
        }
        val s = runCatching { conn.authenticate(auth) }.getOrElse {
            // Sans compte : invité refusé, on tente l'accès anonyme.
            if (login == null || login.user.isBlank()) conn.authenticate(AuthenticationContext.anonymous()) else throw it
        }
        sessions[host] = s
        return s
    }

    @Synchronized
    private fun disk(c: Context, u: Uri): DiskShare {
        val host = u.encodedAuthority.orEmpty()
        val key = "$host/${shareOf(u)}"
        shares[key]?.takeIf { it.isConnected }?.let { return it }
        val d = session(c, host).connectShare(shareOf(u)) as DiskShare
        shares[key] = d
        return d
    }

    /** Vérifie l'accès (et garde les identifiants s'il est accordé). Message d'erreur sinon. */
    fun connect(c: Context, host: String, login: Login): String? = runCatching {
        synchronized(this) { sessions.remove(host)?.let { runCatching { it.close() } } }
        session(c, host, login)
        saveLogin(c, host, login)
        null
    }.getOrElse { it.message ?: it.javaClass.simpleName }

    /** Partages de disque d'un hôte (sans les partages d'administration en `$`). */
    fun shares(c: Context, host: String): List<String> {
        val transport = SMBTransportFactories.SRVSVC.getTransport(session(c, host))
        return ServerService(transport).shares1
            .filter { (it.type and 0xFF) == 0 && !it.netName.endsWith("$") }
            .map { it.netName }
            .sortedBy { it.lowercase() }
    }

    /** Contenu d'un dossier (ou de la racine d'un partage). */
    fun list(c: Context, u: Uri): List<Entry> =
        disk(c, u).list(pathOf(u)).mapNotNull { e ->
            val n = e.fileName
            if (n == "." || n == "..") null
            else Entry(n, e.fileAttributes and 0x10L != 0L, e.endOfFile, e.lastWriteTime.toEpochMillis())
        }

    fun size(c: Context, u: Uri): Long = runCatching {
        disk(c, u).getFileInformation(pathOf(u)).standardInformation.endOfFile
    }.getOrDefault(-1L)

    // --- Lecture --------------------------------------------------------------------------------

    /**
     * Descripteur de lecture sur un fichier distant. Lecture anticipée qui grandit quand la
     * lecture est séquentielle (film : gros blocs, peu d'allers-retours) et reste petite quand elle
     * saute (index des sous-titres : on ne télécharge pas le film pour en lire les en-têtes).
     */
    fun open(c: Context, u: Uri): ParcelFileDescriptor? = runCatching {
        val file = disk(c, u).openFile(
            pathOf(u), EnumSet.of(AccessMask.GENERIC_READ), null, SMB2ShareAccess.ALL,
            SMB2CreateDisposition.FILE_OPEN, null,
        )
        val length = file.fileInformation.standardInformation.endOfFile
        val thread = HandlerThread("smb-read").apply { start() }
        val sm = c.getSystemService(StorageManager::class.java)
        sm.openProxyFileDescriptor(ParcelFileDescriptor.MODE_READ_ONLY, object : ProxyFileDescriptorCallback() {
            private var buf = ByteArray(0)
            private var bufAt = -1L
            private var bufLen = 0
            private var lastEnd = -1L
            private var ahead = MIN_AHEAD

            override fun onGetSize() = length

            override fun onRead(offset: Long, size: Int, data: ByteArray): Int {
                try {
                    var done = 0
                    while (done < size) {
                        val at = offset + done
                        if (at >= length) break
                        if (bufAt < 0 || at < bufAt || at >= bufAt + bufLen) {
                            // Lecture qui avance (même par petits sauts) : blocs de plus en plus gros.
                            val forward = lastEnd >= 0 && at >= lastEnd && at - lastEnd < ahead
                            ahead = if (forward) (ahead * 2).coerceAtMost(MAX_AHEAD) else MIN_AHEAD
                            val want = maxOf(ahead, size - done).toLong().coerceAtMost(length - at).toInt()
                            if (buf.size < want) buf = ByteArray(want)
                            var got = 0
                            while (got < want) {
                                val r = file.read(buf, at + got, got, want - got)
                                if (r <= 0) break
                                got += r
                            }
                            if (got <= 0) break
                            bufAt = at
                            bufLen = got
                        }
                        val from = (at - bufAt).toInt()
                        val n = minOf(size - done, bufLen - from)
                        System.arraycopy(buf, from, data, done, n)
                        done += n
                    }
                    lastEnd = offset + done
                    return done
                } catch (e: Exception) {
                    throw ErrnoException("smb", OsConstants.EIO)
                }
            }

            override fun onRelease() {
                runCatching { file.close() }
                thread.quitSafely()
            }
        }, Handler(thread.looper))
    }.getOrNull()

    private const val MIN_AHEAD = 64 * 1024
    private const val MAX_AHEAD = 4 * 1024 * 1024

    /** Nom affiché d'un dossier ou fichier distant. */
    fun label(u: Uri): String = u.lastPathSegment ?: u.encodedAuthority.orEmpty()
}
