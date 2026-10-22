# Performances : mesures et principes

Ocekour tourne sur des postes parfois lents. Cette page décrit comment le mesurer et les
règles à garder pour qu'il reste fluide.

## Banc de mesure

1. Faux Rancher chargé : `FAKE_HISTORY=100000 FAKE_INTERVAL_MS=10 FAKE_BURST=10 npm run fake-rancher`,
   soit 100 000 lignes d'historique par conteneur et environ 1 000 lignes/s en direct.
2. Lancer l'application (Playwright + Electron), ouvrir `prod › app-commandes › api-commandes… › app`,
   passer « Lignes » à « Tout », puis mesurer :
   - images/s et tâches longues (`PerformanceObserver('longtask')`) pendant 8 s de direct ;
   - images/s pendant un défilement continu ;
   - délai d'une recherche.
3. CPU bridé ×4 (`Emulation.setCPUThrottlingRate` via CDP) pour simuler un poste lent.

Résultats (CPU ×4, 0.2.0 → 0.3.0) :

| Mesure                                     | 0.2.0   | 0.3.0                          |
| ------------------------------------------ | ------- | ------------------------------ |
| Direct ~1 000 lignes/s : images/s          | 7       | ~50                            |
| Direct : temps bloqué sur 8 s              | 7,7 s   | ~0,4 s                         |
| Défilement pendant le direct : images/s    | 5       | ~27 (rendu logiciel sous xvfb) |
| Chargement de 100 000 lignes               | 6,0 s   | 3,7 s                          |
| Recherche (100 000 lignes)                 | 0,95 s  | 0,4 s                          |
| Code de l'interface chargé au démarrage    | 1,35 Mo | 0,48 Mo                        |
| Chargement du processus principal (CPU ×1) | ~500 ms | ~275 ms                        |

## Principes

- **Rien en O(taille du tampon) par lot reçu.** `LogBuffer` retire en tête en O(1) et tient
  les comptes par niveau à jour. `LogRows` filtre seulement le lot reçu. Un filtrage complet
  n'a lieu qu'au changement de filtre, de champs clés ou à la reprise après une pause.
- **Une mise à jour d'affichage par image au plus** (`LogSession.invalidate`,
  `requestAnimationFrame`), quel que soit le débit du flux.
- **Rendu des lignes mis en cache** : chaque ligne est un fragment HTML échappé, calculé
  une fois, au lieu de dizaines de petites vues Angular. Le texte est borné à 400 caractères.
- **Défilement écouté hors d'Angular** : pas de détection de changements par événement.
- **Chargement à la demande** : écrans secondaires (`@defer`), CodeMirror, et côté main
  Kubernetes, Kafka et le serveur MCP (`import()` dynamique). Côté interface, `zod/mini` au
  lieu de zod.
- **Démarrage** : `electron/boot.ts` active le cache de compilation V8 et place les caches
  dans `%LOCALAPPDATA%`. `build/portable.nsi` n'extrait l'application qu'une fois par
  version. `electronLanguages` limite les langues embarquées à fr et en-US.
