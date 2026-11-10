# French wording: proposal for 0.4.2

*Lead engineer, 10 November 2026. Based on the words Anki's French interface uses. You're the native speaker, so change anything in the "Proposed" column, or write "keep" to leave it as it is. Wherever the wording appears elsewhere, the agent changes it consistently, including plurals and screen-reader text.*

| English | Now | Proposed | Why |
|---|---|---|---|
| Time studied, last 30 days / Time Studied per Day | Temps d'étude, … | **Temps passé, 30 derniers jours** / **Temps passé par jour** | The wording you flagged. Anki says "temps passé". |
| {day}: {time} studied | {time} d'étude | **{time} passées** (e.g. "12 min passées") | Same reason |
| Due (state, filter) | À revoir | **À réviser** | Anki's word; it also frees "À revoir" |
| Fail / Pass (buttons) | Raté / Réussi | **Oublié / Su**? or keep | Anki uses "À revoir / Correct" with 4 buttons; for 2 buttons, "Oublié / Su" says what happened in memory. Your call. |
| Failed / Passed (stats legend) | Ratées / Réussies | follows the buttons (**Oubliées / Sues**, or keep) | Consistency |
| Recall Now (card info) | Rappel actuel | **Probabilité de rappel** | "Rappel actuel" sounds odd and clashes with the app's name |
| Expected retention | Rétention attendue | **Rétention estimée** | More natural for an estimate |
| Due Forecast / _Forecast | Prévision des révisions / Prévision | **Révisions à venir** / **À venir** | Anki-like and plain |
| Card Information | Informations sur la carte | **Infos sur la carte** | Anki's wording, shorter on phones |
| Day Starts At | Début de la journée | **Le jour commence à** | Reads as a sentence, like Anki's "Le jour suivant commence à" |
| Reviews per Day (limit) | Révisions par jour | **Révisions max. par jour** | Says it's a limit, as Anki does |
| Study (button) | Étudier | keep | Anki: "Étudier maintenant"; the short form fits the button |
| Session Complete | Séance terminée | keep | Natural |
| Unsuspend | Réactiver | keep | Clearer than Anki's "Ne plus suspendre" |
| Deck, Tag, Front/Back, Lapses, Stability | Paquet, Étiquette, Recto/Verso, Oublis, Stabilité | keep | Already Anki's words |

The other ~800 strings stay as they are unless you send more. The agent also checks every changed string at 360 px (French is longer).
