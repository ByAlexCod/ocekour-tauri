# Notifications GitLab, Jira, Confluence vers Rocket.Chat

[← Sommaire](README.md)

Depuis la 0.4.0, Ocekour surveille **GitLab**, **Jira** et **Confluence** et, dès qu'il y a
du nouveau **qui vous concerne directement**, vous envoie **un seul message groupé dans
Rocket.Chat** — à vous-même par défaut. Il réutilise les jetons du [hub MCP](hub-mcp.md).

## Activer

**Paramètres → Notifications** → cochez _Surveiller GitLab, Jira et Confluence et m'envoyer
les nouveautés dans Rocket.Chat_.

- Chaque source n'est surveillée que si elle est **cochée ici** ET a un **jeton**
  (Paramètres → Comptes). Rocket.Chat doit aussi avoir un jeton : c'est lui qui vous écrit.
- **Intervalle** : toutes les 60 s par défaut (réglable, minimum 30 s).
- **Destinataire** : `self` (à vous-même) par défaut ; sinon un canal `#nom` ou une personne
  `@nom`.
- **Envoyer un message d'essai** vérifie l'envoi tout de suite.
- Au **premier lancement**, Ocekour envoie un message de confirmation et part de « maintenant »
  (il ne déverse pas l'historique).

## Ce qui déclenche une notification

Seulement ce qui vous concerne directement :

| Source         | Événements                                                                                                                                                                             |
| -------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **GitLab**     | assigné, revue demandée, mention, **pipeline en échec**, approbation demandée, MR non mergeable, **commentaire sur une MR que vous avez écrite**, **réponse à un de vos commentaires** |
| **Jira**       | assigné, commentaire sur vos tickets, mention, changement de statut                                                                                                                    |
| **Confluence** | mention, commentaire ou modification sur une page que vous suivez                                                                                                                      |

Les actions que **vous** faites vous-même ne déclenchent rien.

Le message regroupe tout par source, avec un lien vers chaque élément :

```
🔔 Ocekour — 2 nouveautés

GitLab
• Revue demandée — grp/app !42 « Paiement » · @amartin

Jira
• Commentaire — PAY-123 « Double paiement » · @bob
```

## Notifications Windows

Les nouveautés s'affichent aussi en **notifications système** (coin de l'écran Windows), en
plus du message Rocket.Chat. Décochez _Afficher aussi les nouveautés en notifications Windows_
dans Paramètres → Notifications pour n'avoir que Rocket.Chat. Un clic sur la notification
ramène Ocekour au premier plan. Elles fonctionnent aussi en mode `--notify` (sans fenêtre).

> Les notifications Windows fonctionnent même si Rocket.Chat est momentanément injoignable :
> les deux canaux sont indépendants.

## Sans interface (démon)

Ocekour n'est **pas un serveur** : les notifications ne partent que **pendant qu'il tourne**.
Vous pouvez le laisser tourner **sans fenêtre**, avec seulement les notifications :

```
Ocekour.exe --notify
```

À lancer au démarrage de la session (raccourci ou tâche planifiée). Il utilise la même
configuration, les mêmes comptes et la même clé d'activation. Ce mode peut tourner en même
temps que l'interface ou que le serveur MCP (`--mcp-http`).

## Bon à savoir

- **Aucun secret** ne circule : les jetons restent chiffrés sur le poste ; seuls les titres
  et les liens partent dans votre message Rocket.Chat.
- Les notifications **n'empruntent pas** le chemin d'écriture des agents : elles ne
  demandent pas « autoriser l'écriture » et n'apparaissent pas dans « Actions de l'agent ».
- **Anti-doublon** : Ocekour retient ce qu'il a déjà envoyé (`notifications.json`) ; rien
  n'est envoyé deux fois, même après un redémarrage.
- **Rocket.Chat injoignable** : les nouveautés sont regroupées et envoyées au passage suivant.
- **Commentaires GitLab** : « commentaire sur votre MR » et « réponse à votre commentaire »
  sont détectés sur les MR que vous avez **écrites** ou dont vous êtes **relecteur**. Une
  réponse à un commentaire que vous avez laissé sur la MR d'un autre (sans être relecteur)
  peut être manquée. Les mentions restent couvertes par la boîte To-Do.
- **Mentions Jira** : Jira Data Center n'a pas de recherche « mentionné ». Ocekour repère les
  mentions dans les commentaires des tickets qui vous concernent déjà ; une mention isolée
  sur un ticket où vous n'apparaissez pas autrement peut passer inaperçue.
