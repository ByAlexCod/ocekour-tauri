# Spec — Outils MCP « Web / navigateur » (Playwright) (1.2.0)

> Statut : **implémentée en 1.2.0** · Guide utilisateur : [web.md](../guide/web.md)
> · S'appuie sur le hub MCP existant (`electron/mcp/`), le service HTTP TLS et turndown.

## 1. Objectif

Exposer aux agents locaux (Qwen Code, Claude Code, Vibe…), via le **serveur MCP d'Ocekour**, un
**accès au web** et un **contrôle de navigateur** : ouvrir des pages, lire leur contenu, cliquer,
remplir des formulaires, capturer l'écran. Cas d'usage : un agent qui consulte une doc interne,
reproduit un parcours, relève une valeur affichée dans une IHM.

## 2. État de l'art open-source

La référence est **Microsoft Playwright MCP** (`@playwright/mcp`). Parti-pris repris :

- **Snapshot d'accessibilité** plutôt que screenshots+coordonnées : un arbre d'éléments
  (rôle + nom), chacun avec une **ref** stable. Les actions ciblent une ref → déterministe.
- Après chaque action, on **renvoie un nouveau snapshot** de la page à jour.
- Screenshots gardés en **complément** (web_capture), pas comme moyen principal d'action.

Différence d'implémentation : plutôt que de dépendre des sélecteurs internes `aria-ref` de
Playwright (expérimentaux, sensibles à la version), Ocekour **marque lui-même** les éléments
interactifs dans la page (`data-ocekour-ref="eN"`) via `page.evaluate`, et agit par
`[data-ocekour-ref="eN"]`. 100 % API publique de Playwright, stable entre versions.

## 3. Navigateur piloté (choix retenu)

- **Navigateur du poste** (Edge par défaut, ou Chrome), piloté par **`playwright-core`** avec
  `channel: 'msedge' | 'chrome'`. **Aucun binaire de navigateur téléchargé** (indispensable en
  environnement cloisonné) : on réutilise le navigateur déjà installé. L'exe reste léger.
- `playwright-core` est une **dépendance de production**, chargée **paresseusement** au premier
  usage via `createRequire` (comme les autres dépendances facultatives). Marquée `external` du
  bundle esbuild, embarquée par electron-builder (`files` + `asarUnpack`).
- Fenêtre **visible par défaut** (l'utilisateur voit ce que fait l'agent) ; `headless` en option.

## 4. Configuration (`config.yaml`)

```ts
web: {
  enabled: boolean;         // défaut false — les outils web_* n'apparaissent que si true
  browser: 'msedge' | 'chrome' | 'chromium';  // défaut 'msedge'
  headless: boolean;        // défaut false (fenêtre visible)
  timeoutMs: int;           // défaut 30000 — navigation / actions
  tls: { insecure, caFile }; // web_recuperer + ignoreHTTPSErrors du navigateur
}
```

Désactivé par défaut (comme le contexte IA). Les outils ne sont visibles que si `web.enabled`
**et** la licence active.

## 5. Outils (préfixe `web_`)

| Outil | Rôle | Lecture seule |
| ----- | ---- | :-: |
| `web_recuperer(url)` | **Sans navigateur** : GET HTTP(S) → Markdown (ou texte) | ✓ |
| `web_naviguer(url, nouvel_onglet?)` | Ouvre/charge une page ; renvoie le snapshot | |
| `web_apercu()` | Snapshot d'accessibilité (refs) de la page active | ✓ |
| `web_texte()` | Contenu de la page active en Markdown | ✓ |
| `web_cliquer(ref, description?)` | Clique l'élément `ref` ; renvoie le snapshot | |
| `web_saisir(ref, texte, valider?)` | Remplit un champ ; `valider` → Entrée | |
| `web_choisir(ref, valeurs[])` | Choisit des options d'un `<select>` | |
| `web_touche(touche)` | Touche clavier (`Enter`, `Escape`, `PageDown`…) | |
| `web_patienter(texte?, ms?)` | Attend un texte à l'écran ou un délai | ✓ |
| `web_capture(pleine_page?)` | Capture d'écran (image PNG) | ✓ |
| `web_onglets()` | Liste les onglets (index, titre, URL) | ✓ |
| `web_fermer(onglet?)` | Ferme un onglet (index) ou tout le navigateur | |

Le navigateur et les onglets **persistent** entre les appels MCP (le hub vit dans le contexte
partagé, pas dans le serveur par requête). Lancement **paresseux** au premier `web_naviguer`.

## 6. Snapshot

`page.evaluate` parcourt le DOM, retient les éléments **interactifs ou porteurs de sens**
(liens, boutons, champs, `select`, `[role]`, titres, éléments cliquables), calcule un **nom
accessible** (texte / `aria-label` / `placeholder` / `alt` / `title`), pose `data-ocekour-ref`
et renvoie `{ ref, role, name, value? }[]`. Le **formatage** en texte lisible est une fonction
**pure** (testable sans navigateur). Les actions résolvent `ref` → `[data-ocekour-ref="eN"]`.

## 7. Sécurité / cloisonnement

- `web_*` **désactivés par défaut** ; le serveur MCP est déjà en 127.0.0.1 + jeton.
- TLS : `tls.insecure` / `caFile` comme les autres services (CA d'entreprise, https auto-signé) ;
  le navigateur reçoit `ignoreHTTPSErrors: tls.insecure`.
- Aucun secret ; les captures et contenus ne transitent que par le MCP local.
- Le journal d'actions enregistre les navigations (comme les écritures de service). _(à confirmer)_

## 8. Découpage (tout en 1.2.0)

1. Schéma `web` (`shared/config.ts`) + types (`shared/api.ts`).
2. `electron/web/` : `fetch.ts` (GET + html→markdown), `snapshot.ts` (walker + formatage pur),
   `browser.ts` (hub Playwright paresseux), `tools.ts` (`registerWebTools`).
3. `content.ts` : bloc image ; `McpContext.webHub` ; instructions du serveur.
4. Empaquetage : dépendance `playwright-core`, `external` esbuild, `files` + `asarUnpack`.
5. Tests (fetch html→markdown, formatage snapshot, validation d'args), guide, CHANGELOG, version.

## 9. Hors périmètre 1.2.0 (plus tard)

- Écran **Paramètres → Web** (toggle, navigateur, headless) : pour l'instant via `config.yaml`.
- Mode `--vision` (clics par coordonnées), téléchargements, upload de fichiers, PDF.
- Réutilisation du **profil connecté** du navigateur (cookies de session) : à cadrer (sécurité).
