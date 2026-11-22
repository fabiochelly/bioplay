package app.bioplay

/** Cœur Rust (`core/`) : index de tous les sous-titres d'un fichier. */
object Core {
    init {
        System.loadLibrary("bioplay_core")
    }

    /** Prend possession de [fd] et le ferme. Format : voir `core/src/model.rs`. */
    @JvmStatic
    external fun index(fd: Int): String

    /** Instants (s) des images clés (index du MKV ; vide sinon). Prend possession de [fd]. */
    @JvmStatic
    external fun keyframes(fd: Int): DoubleArray
}

class Cue(val start: Long, val end: Long, val text: String)

class SubIndex(
    val number: Long,
    val codec: String,
    val lang: String,
    val name: String,
    val image: Boolean,
    val cues: List<Cue>,
) {
    private val starts = LongArray(cues.size) { cues[it].start }

    /** Dernière ligne commencée à [ms], ou -1. */
    fun at(ms: Long): Int {
        var lo = 0
        var hi = starts.size - 1
        var found = -1
        while (lo <= hi) {
            val mid = (lo + hi) ushr 1
            if (starts[mid] <= ms) {
                found = mid; lo = mid + 1
            } else hi = mid - 1
        }
        return found
    }

    /** Ligne visée par [steps] crans depuis [ms] : +1 la suivante, -1 la précédente. */
    fun step(ms: Long, steps: Int): Int? {
        if (cues.isEmpty() || steps == 0) return null
        // Une ligne commencée il y a moins de 0,8 s compte comme « déjà passée » vers l'arrière.
        val base = at(ms + 40)
        val target = if (steps > 0) base + steps else {
            val current = if (base >= 0 && ms - starts[base] < 800) base - 1 else base
            current + steps + 1
        }
        return target.coerceIn(0, cues.size - 1)
    }

    companion object {
        fun parse(s: String): List<SubIndex> {
            val out = ArrayList<SubIndex>()
            var head: List<String>? = null
            var cues = ArrayList<Cue>()
            fun flush() {
                val h = head ?: return
                out += SubIndex(h[1].toLong(), h[2], h[3], h[4], h[5] == "1", cues)
            }
            for (line in s.lineSequence()) {
                if (line.isEmpty()) continue
                val f = line.split('\t', limit = if (line[0] == 'C') 4 else 6)
                if (f[0] == "T") {
                    flush(); head = f; cues = ArrayList()
                } else if (f.size == 4) {
                    cues += Cue(f[1].toLong(), f[2].toLong(), f[3].replace('\u001f', '\n'))
                }
            }
            flush()
            return out
        }
    }
}
