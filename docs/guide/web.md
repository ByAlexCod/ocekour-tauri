# Web & navigateur : outils MCP pour vos agents

[← Sommaire](README.md)

Depuis la 1.2.0, Ocekour peut donner à vos agents (via le serveur MCP) un **accès au web** et le
**contrôle d'un navigateur**. Ocekour pilote le navigateur **déjà installé sur votre poste**
(Microsoft Edge par défaut, ou Chrome) par son protocole DevTools — **aucun navigateur n'est
téléchargé**, ce qui convient aux postes **cloisonnés**.

> Désactivé par défaut. Les outils `web_*` n'apparaissent que si vous l'activez **et** que le
> serveur MCP est en marche.

## Activer

Dans `config.yaml` (Paramètres → bouton **Ouvrir config.yaml**) :

```yaml
web:
  enabled: true        # active les outils web_*
  browser: msedge      # msedge | chrome | chromium
  headless: false      # false = fenêtre visible (vous voyez l'agent agir)
  timeoutMs: 30000      # délai de navigation / action
  tls:
    insecure: false    # true = ignorer les erreurs de certificat (https interne)
    # caFile: C:\chemin\vers\ca.crt
```

Il faut que **Edge** (ou **Chrome**) soit installé sur le poste.

## Les outils exposés

Le modèle suit **Microsoft Playwright MCP** : on travaille par **snapshot d'accessibilité**.
Chaque élément de la page reçoit une **ref** (`e1`, `e2`…) que l'agent cible pour agir — pas de
« clic au pixel ». Après chaque action, Ocekour renvoie un snapshot à jour.

| Outil | Rôle |
| ----- | ---- |
| `web_recuperer(url)` | **Sans navigateur** : lit une URL et la renvoie en Markdown (idéal pour une doc) |
| `web_naviguer(url, nouvel_onglet?)` | Ouvre une page dans le navigateur ; renvoie le snapshot |
| `web_apercu()` | Snapshot d'accessibilité de la page active |
| `web_texte()` | Contenu texte de la page active |
| `web_cliquer(ref)` | Clique l'élément `ref` |
| `web_saisir(ref, texte, valider?)` | Remplit un champ (`valider` → Entrée) |
| `web_choisir(ref, valeurs)` | Choisit des options d'une liste déroulante |
| `web_touche(touche)` | Touche clavier (`Enter`, `Escape`, `PageDown`…) |
| `web_patienter(texte?, ms?)` | Attend un texte à l'écran ou un délai |
| `web_capture(pleine_page?)` | Capture d'écran (image) |
| `web_onglets()` | Liste les onglets ouverts |
| `web_fermer(onglet?)` | Ferme un onglet, ou tout le navigateur |

Le navigateur et ses onglets **restent ouverts entre les appels** : l'agent enchaîne
naviguer → apercu → cliquer → saisir sans tout relancer. Il se lance au premier `web_naviguer`.

## Bon à savoir

- `web_recuperer` ne lance pas de navigateur : rapide pour une page statique ou une API. Pour une
  page qui exige JavaScript ou une interaction, passez par `web_naviguer`.
- TLS : `insecure` / `caFile` fonctionnent comme pour les autres services (CA d'entreprise,
  certificat auto-signé).
- Tout reste local : le serveur MCP n'écoute que sur `127.0.0.1` derrière un jeton ; aucune
  donnée n'est envoyée ailleurs que vers les sites que l'agent visite.
