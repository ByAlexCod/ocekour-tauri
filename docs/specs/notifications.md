# Spec — Notifications vers Rocket.Chat (0.4.0)

> Statut : **validée, implémentée en 0.4.0** · Guide : [notifications.md](../guide/notifications.md)
> · Réutilise les comptes du [hub MCP](../guide/hub-mcp.md).

## 1. Objectif

Aujourd'hui, les notifications de **GitLab**, **Jira** et **Confluence** ne ressortent nulle
part dans le poste de travail. Ocekour interroge régulièrement ces trois services et, dès
qu'il y a du nouveau **qui concerne directement l'utilisateur**, il envoie **un seul message
Rocket.Chat** (regroupé) **à soi-même**.

Principes :

- **Ça me concerne directement** uniquement (assigné, mentionné, revue demandée…), pas tout
  ce que je suis.
- **Un message groupé par passage**, pas un message par événement.
- **Aucun secret** : réutilise les jetons du hub (GitLab, Jira, Confluence, Rocket.Chat),
  déjà chiffrés dans `secrets.json`. `config.yaml` ne porte que les réglages.
- Ocekour n'est **pas un serveur** : les notifications ne partent que **pendant qu'il
  tourne** (interface ouverte, ou sans interface). PC éteint → rien pendant ce temps, on
  rattrape au rallumage (voir §6, anti-doublon).
- Fonctionnalité **entièrement côté A** (la machine qui voit GitLab / Jira / Confluence /
  Rocket.Chat) ; indépendante du RDP A↔B.

Périmètre 0.4.0 : **les trois sources en même temps**. Hors périmètre : notifications
Rancher / Kafka / S3, canaux multiples, destinataires multiples, résumé quotidien.

## 2. Comment lire les notifications de chaque source

Il n'existe pas d'API unique « mes notifications ». Stratégie par source (à confirmer sur vos
instances) :

### 2.1 GitLab — API To-Do (propre)

`GET /api/v4/todos?state=pending` : la vraie boîte de notifications, déjà qualifiée.

| `action_name`                     | Événement                   | Retenu par défaut |
| --------------------------------- | --------------------------- | ----------------- |
| `assigned`                        | Assigné à une issue / MR    | ✅                |
| `review_requested`                | Revue de MR demandée        | ✅                |
| `mentioned`, `directly_addressed` | Mentionné (`@moi`)          | ✅                |
| `build_failed`                    | Pipeline en échec sur ma MR | ✅                |
| `approval_required`               | Mon approbation demandée    | ✅                |
| `unmergeable`                     | MR non mergeable            | ✅                |
| `marked`, `merge_train_removed`   | Autres                      | ❌ (option)       |

Chaque to-do donne `id`, `target_type`, `target` (titre, `iid`, `web_url`), `project`,
`author`, `body`, `created_at`, `target_url`. **Anti-doublon** : `id` croissant. On **ne
marque pas** les to-do comme lus (on ne touche pas à la boîte de l'utilisateur).

### 2.2 Jira Data Center — JQL + changelog

Pas d'endpoint « notifications ». On interroge par **JQL** les tickets qui me concernent et
qui ont bougé depuis le dernier passage, puis on lit le **changelog / les commentaires** pour
décrire _quoi_ a changé et écarter mes propres actions.

```
(assignee = currentUser() OR reporter = currentUser() OR watcher = currentUser())
AND updated >= "<dernier passage>" ORDER BY updated ASC
```

Pour chaque ticket : `GET /rest/api/2/issue/<clé>?expand=changelog` (ou
`/comment?startAt=…`), on garde les changements **postérieurs au curseur** et **faits par
quelqu'un d'autre que moi** :

| Événement                                | Retenu par défaut                  |
| ---------------------------------------- | ---------------------------------- |
| Ticket assigné à moi                     | ✅                                 |
| Commentaire sur un de mes tickets        | ✅                                 |
| Changement de statut d'un de mes tickets | ✅                                 |
| Mention `@moi` dans un commentaire       | ✅ si détectable (voir ci-dessous) |

> **Mentions Jira** : Jira DC n'a pas de JQL « mentionné ». On repère le motif de mention
> (`[~identifiant]` en wiki-markup) dans les commentaires des tickets déjà remontés par le
> JQL. Une mention sur un ticket où je n'apparais pas autrement (ni assigné, ni watcher)
> peut manquer. **À confirmer sur votre instance** : si le flux d'activité ATOM (`/activity`)
> porte les mentions de façon fiable, on l'ajoutera comme source complémentaire.

**Anti-doublon** : couple `clé + horodatage du changement`.

### 2.3 Confluence DC — CQL

Deux requêtes CQL depuis le dernier passage :

```
mention = currentUser() AND lastModified > "<dernier passage>"
watcher = currentUser() AND lastModified > "<dernier passage>"
```

| Événement                                 | Retenu par défaut |
| ----------------------------------------- | ----------------- |
| Mentionné dans une page ou un commentaire | ✅                |
| Commentaire sur une page que je suis      | ✅                |
| Page que je suis modifiée (par un autre)  | ✅                |

On écarte les modifications dont l'auteur est moi. **Anti-doublon** : `id de contenu +
numéro de version` (ou horodatage).

## 3. Le message Rocket.Chat

**Un seul message par passage**, à **soi-même** (auto-message Rocket.Chat ; le nom
d'utilisateur vient de `GET /api/v1/me`). Destinataire **réglable** (`self`, `#canal` ou
`@personne`), `self` par défaut.

Format Markdown, groupé par source, avec liens :

```
🔔 **Ocekour — 3 nouveautés**

**GitLab**
• Revue demandée — [groupe/app !42 « Paiement »](https://gitlab.intra/…) · par @amartin
• Pipeline en échec — [groupe/app !40](https://gitlab.intra/…)

**Jira**
• Commentaire — [PAY-123 « Double paiement »](https://jira.intra/browse/PAY-123) · @bob

**Confluence**
• Mention — [Déploiement prod](https://confluence.intra/…) · @claire
```

- Un en-tête `🔔 Ocekour` identifie l'expéditeur (message à soi-même).
- La notification **n'emprunte pas** le chemin d'écriture des agents : elle n'exige pas
  « autoriser l'écriture » (§ hub MCP) et **n'apparaît pas** dans « Actions de l'agent »
  (ce n'est pas un agent). Elle utilise directement le compte Rocket.Chat.
- Rien de secret ; les titres et libellés viennent des services.

## 4. Configuration (`config.yaml`)

Nouvelle section `notifications`, sans aucun secret :

```yaml
notifications:
  enabled: true
  intervalSeconds: 60 # réglable (min 30)
  target: self # self | "#canal" | "@personne"
  quietHours: { from: '20:00', to: '08:00' } # facultatif : rien pendant ces heures
  sources:
    gitlab: { enabled: true } # events: liste facultative (défaut : §2.1)
    jira: { enabled: true }
    confluence: { enabled: true }
```

- Une source n'est active que si elle est **activée ici** ET a un **jeton** (écran Comptes).
- Réglé dans un nouvel écran **Paramètres → Notifications** (interrupteur global, intervalle,
  destinataire, heures calmes, une carte par source avec ses types d'événements) ; un bouton
  **« Tester »** envoie un message d'exemple.

## 5. État local (anti-doublon)

Fichier `%APPDATA%\Ocekour\notifications.json` (état, ni config ni secret) :

```jsonc
{
  "cursors": {
    // dernier passage pris en compte, par source
    "gitlab": { "lastId": 98765 },
    "jira": { "since": "2026-09-25T09:00:00.000Z" },
    "confluence": { "since": "2026-09-25T09:00:00.000Z" },
  },
  "seen": ["gitlab:98765", "jira:PAY-123@2026-…", "…"], // fenêtre récente, anti-doublon
  "pending": [], // nouveautés non encore envoyées (Rocket.Chat injoignable)
}
```

- **Premier lancement** : les curseurs partent de **maintenant** ; aucun déversement de
  l'historique. (Option : un message « notifications activées » de confirmation.)
- **Recouvrement** : on interroge depuis `curseur − marge` (quelques minutes) pour ne rien
  rater à cheval sur un passage, et `seen` évite les doublons.
- **Livraison** : le curseur d'une source avance dès qu'elle est lue ; les nouveautés vont
  dans `pending`. Si Rocket.Chat est injoignable, `pending` est **regroupé au passage
  suivant** (borné, ex. 50 éléments, puis résumé « … et N autres »).

## 6. Cycle de fonctionnement

- Un **scrutateur** (`Notifier`) démarre avec l'application quand `notifications.enabled`
  et que la licence est active. Il tourne **en mode interface** et **sans interface**
  (`--mcp-http`). Un mode dédié **`--notify`** (sans interface, sans MCP) est proposé pour
  un « démon de notifications » seul.
- À chaque tick (intervalle réglé, hors heures calmes) : interroger les sources activées en
  parallèle, construire le lot, envoyer un message, avancer les curseurs, écrire l'état.
- **Erreurs** : une source en échec (jeton expiré, réseau) est **ignorée pour ce tick**,
  journalisée (journal technique), et retentée au suivant ; un jeton expiré ajoute une
  nouveauté « ⚠️ Jeton Jira expiré — Paramètres → Comptes » **une seule fois**.
- **Matomo** : catégorie `Notifications`, action par source, valeur = nombre envoyé ; jamais
  de contenu ni d'URL.

## 7. Implémentation proposée

```
electron/notifications/
  notifier.ts        boucle (intervalle, heures calmes), regroupe et envoie, gère l'état
  state.ts           notifications.json : curseurs, anti-doublon, pending
  message.ts         mise en forme du message groupé (Markdown, liens)
  gitlab.ts          to-do → nouveautés
  jira.ts            JQL + changelog → nouveautés
  confluence.ts      CQL → nouveautés
shared/config.ts     NotificationsSchema (+ preset.ts : optionnel, non prioritaire)
electron/main.ts     démarrage du Notifier (interface / --mcp-http / --notify)
src/app/settings/notifications.ts   écran Paramètres → Notifications
```

- Réutilise le **client HTTP** et les **comptes** du hub (`electron/services/`), sans
  toucher au chemin d'écriture des agents ni au journal des actions.
- Envoi Rocket.Chat via le compte Rocket.Chat directement (auto-message).

## 8. Tests

- **Faux serveurs** (sur le modèle de `fake-services`) : to-do GitLab, tickets Jira avec
  changelog, contenus Confluence CQL, et Rocket.Chat qui enregistre le message envoyé.
- Cas couverts : premier lancement (rien envoyé), une nouveauté par source, **anti-doublon**
  sur recouvrement, changement fait **par moi** ignoré, **heures calmes**, Rocket.Chat
  injoignable → regroupé au tick suivant, jeton expiré signalé une fois.
- Mise en forme du message (groupement, liens, borne).
- Intégration facultative sur vos vraies instances (variables d'environnement), à lancer en
  local — vous validez notamment la détection des **mentions Jira** (§2.2).

## 9. Décisions

1. **Mentions Jira** : détectées dans les commentaires des tickets remontés par le JQL
   (motif `[~identifiant]`). Une mention sur un ticket où l'utilisateur n'apparaît pas
   autrement peut manquer — limite acceptée pour la v1.
2. **Message de confirmation** au premier passage : oui.
3. **Heures calmes** : abandonnées (non retenues).
4. **`--notify`** (démon sans interface, sans MCP) : livré.
