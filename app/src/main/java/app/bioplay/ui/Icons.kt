package app.bioplay.ui

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.PathFillType
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.PathParser
import androidx.compose.ui.unit.dp

/** Icônes dessinées ici (tracés Material, 24×24) : pas de bibliothèque d'icônes à embarquer. */
object Icons {
    private fun icon(name: String, vararg paths: String) = ImageVector.Builder(name, 24.dp, 24.dp, 24f, 24f).apply {
        paths.forEach { addPath(PathParser().parsePathString(it).toNodes(), fill = SolidColor(Color.White)) }
    }.build()

    val play = icon("play", "M8 5.14v13.72a1 1 0 0 0 1.52.85l10.6-6.86a1 1 0 0 0 0-1.7L9.52 4.29A1 1 0 0 0 8 5.14z")
    val pause = icon("pause", "M7 4.5h3a1 1 0 0 1 1 1v13a1 1 0 0 1-1 1H7a1 1 0 0 1-1-1v-13a1 1 0 0 1 1-1zm7 0h3a1 1 0 0 1 1 1v13a1 1 0 0 1-1 1h-3a1 1 0 0 1-1-1v-13a1 1 0 0 1 1-1z")
    val back = icon("back", "M15.7 5.3a1 1 0 0 1 0 1.4L10.4 12l5.3 5.3a1 1 0 0 1-1.4 1.4l-6-6a1 1 0 0 1 0-1.4l6-6a1 1 0 0 1 1.4 0z")
    val prevLine = icon("prev", "M6 5h2v14H6zm3.5 7l8.5 6V6z")
    val nextLine = icon("next", "M16 5h2v14h-2zM6 18l8.5-6L6 6z")
    val tracks = icon(
        "tracks",
        "M20 4H4a2 2 0 0 0-2 2v12a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2V6a2 2 0 0 0-2-2zm0 14H4V6h16v12z",
        "M6 10h2v4H6zm3-2h2v8H9zm3 3h2v2h-2zm3-1h3v1.6h-3zm0 2.4h3V16h-3z",
    )
    val speed = icon("speed", "M20.38 8.57l-1.23 1.85a8 8 0 0 1-.22 7.58H5.07A8 8 0 0 1 15.58 6.85l1.85-1.23A10 10 0 0 0 3.35 19a2 2 0 0 0 1.72 1h13.85a2 2 0 0 0 1.74-1 10 10 0 0 0-.27-10.44zm-9.79 6.84a2 2 0 0 0 2.83 0l5.66-8.49-8.49 5.66a2 2 0 0 0 0 2.83z")
    val zoom = icon("zoom", "M7 14H5v5h5v-2H7v-3zm-2-4h2V7h3V5H5v5zm12 7h-3v2h5v-5h-2v3zM14 5v2h3v3h2V5h-5z")
    val list = icon("list", "M3 13h2v-2H3v2zm0 4h2v-2H3v2zm0-8h2V7H3v2zm4 4h14v-2H7v2zm0 4h14v-2H7v2zM7 7v2h14V7H7z")
    val add = icon("add", "M19 13h-6v6h-2v-6H5v-2h6V5h2v6h6v2z")
    val minus = icon("minus", "M19 13H5v-2h14v2z")
    val check = icon("check", "M9 16.17L4.83 12l-1.42 1.41L9 19 21 7l-1.41-1.41z")
    val folder = icon("folder", "M20 6h-8l-2-2H4a2 2 0 0 0-2 2v12a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2zm0 12H4V8h16v10z")
    val speaker = icon("speaker", "M3 9v6h4l5 5V4L7 9H3zm13.5 3c0-1.77-1.02-3.29-2.5-4.03v8.05c1.48-.73 2.5-2.25 2.5-4.02zM14 3.23v2.06c2.89.86 5 3.54 5 6.71s-2.11 5.85-5 6.71v2.06c4.01-.91 7-4.49 7-8.77s-2.99-7.86-7-8.77z")
    /** Sous-titres : cadre arrondi à bord fin, deux lignes de texte en pastilles. */
    val subtitles = ImageVector.Builder("subtitles", 24.dp, 24.dp, 24f, 24f).apply {
        val frame = "M5.5 4h13A3.5 3.5 0 0 1 22 7.5v9a3.5 3.5 0 0 1-3.5 3.5h-13A3.5 3.5 0 0 1 2 16.5v-9A3.5 3.5 0 0 1 5.5 4z" +
            "M5.5 5.7A1.8 1.8 0 0 0 3.7 7.5v9a1.8 1.8 0 0 0 1.8 1.8h13a1.8 1.8 0 0 0 1.8-1.8v-9a1.8 1.8 0 0 0-1.8-1.8z"
        addPath(PathParser().parsePathString(frame).toNodes(), fill = SolidColor(Color.White), pathFillType = PathFillType.EvenOdd)
        val lines = "M6.7 11.4h6.8a.8.8 0 0 1 0 1.6H6.7a.8.8 0 0 1 0-1.6z" + "M15.3 11.4h2a.8.8 0 0 1 0 1.6h-2a.8.8 0 0 1 0-1.6z" +
            "M6.7 14.6h2.6a.8.8 0 0 1 0 1.6H6.7a.8.8 0 0 1 0-1.6z" + "M11.1 14.6h6.2a.8.8 0 0 1 0 1.6h-6.2a.8.8 0 0 1 0-1.6z"
        addPath(PathParser().parsePathString(lines).toNodes(), fill = SolidColor(Color.White))
    }.build()
    val subtitlesOff = icon("subtitles-off", "M20 4H6.83l8 8H20v2h-3.17l4.93 4.93c.15-.28.24-.59.24-.93V6c0-1.1-.9-2-2-2zM1.04 3.87l1.2 1.2C2.09 5.35 2 5.66 2 6v12c0 1.1.9 2 2 2h13.17l2.96 2.96 1.41-1.41L2.45 2.45 1.04 3.87zM8 12v2H4v-2h4zm6 4.83V18H4v-2h9.17l.83.83z")
    /** Ouvrir : une feuille au coin plié, un triangle de lecture dedans. */
    val open = icon("open", "M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8l-6-6zM6 20V4h7v5h5v11H6z", "M10 11.2v6.6l5.2-3.3z")
    val grid = icon("grid", "M4 4h7v7H4zm9 0h7v7h-7zM4 13h7v7H4zm9 0h7v7h-7z")
    val rows = icon("rows", "M3 5h5v4H3zm7 .5h11v3H10zM3 10h5v4H3zm7 .5h11v3H10zM3 15h5v4H3zm7 .5h11v3H10z")
    val close = icon("close", "M19 6.41L17.59 5 12 10.59 6.41 5 5 6.41 10.59 12 5 17.59 6.41 19 12 13.41 17.59 19 19 17.59 13.41 12z")
}
