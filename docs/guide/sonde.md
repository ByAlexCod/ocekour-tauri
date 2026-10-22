# Sonde : les logs d'une machine distante

[← Sommaire](README.md)

Beaucoup de logs ne sont ni dans Rancher ni dans Kafka : VM, serveurs Windows ou Linux,
machines de recette. La **sonde** les expose **en lecture seule** aux postes Ocekour : plus
besoin de RDP ou de SSH pour lire un fichier.

- C'est `ocekour-headless.exe` (Windows) ou `ocekour-headless` (Linux : binaire statique, sans
  dépendance), lancé **sur la machine distante**, sans interface.
- Seules les sources **déclarées par l'administrateur** sont lisibles.
- La sonde **écoute et répond** ; elle n'appelle jamais personne. Seul son port est à ouvrir,
  en entrée.
- Plusieurs personnes peuvent la lire en même temps, chacune avec ses options.
- Elle ne demande pas de clé d'activation et n'envoie aucune statistique.

## 1. Installer la sonde (administrateur de la machine)

Copiez l'exécutable sur la machine, puis :

```
ocekour-headless sonde init --ecoute=0.0.0.0:47120 --nom=srv-recette-01
```

Cela crée, dans le dossier courant (ou `--dossier=…`) : `sonde.yaml`, un certificat
autosigné (`sonde.crt`, `sonde.key`) et un **jeton** d'accès. La commande affiche ce qu'il
faut transmettre aux utilisateurs :

```
  Adresse    : https://srv-recette-01:47120
  Empreinte  : SHA256:9F:2C:…:A1
  Jeton      : osk_3bb1e0c4…
```

Le jeton n'est affiché **qu'une fois** (seule son empreinte est gardée dans `sonde.yaml`) :
transmettez-le de façon sûre.

### Déclarer les sources

Dans `sonde.yaml` :

```yaml
sources:
  - id: api-commandes          # [a-z0-9-], stable : liens et favoris en dépendent
    nom: API commandes
    fichier: /var/log/commandes/api.log
  - id: batch-nuit
    motif: /var/log/batch/batch-*.log   # jokers dans le nom seulement
    garder: 30
  - id: iis
    motif: C:\inetpub\logs\LogFiles\W3SVC1\u_ex*.log
    encodage: windows-1252     # utf-8 (défaut), utf-16, latin-1, windows-1252
limites:
  flux_simultanes: 50          # au total
  flux_par_poste: 10
```

La sonde recharge `sonde.yaml` à chaud ; aucun redémarrage n'est nécessaire.

### Vérifier, démarrer

```
ocekour-headless sonde verifier     # relit la config, teste chaque source (code 1 si un problème)
ocekour-headless sonde              # démarre au premier plan (Ctrl+C pour arrêter)
```

### En service

```
ocekour-headless sonde service installer [--utilisateur=<compte>]
ocekour-headless sonde service desinstaller
```

- **Windows** : service « Ocekour Sonde », démarrage automatique. À lancer dans une invite
  administrateur.
- **Linux** : unité systemd générée (`ocekour-sonde.service`), en lecture seule sur le disque.

Donnez au compte du service **la lecture des dossiers de logs, et rien d'autre** (pas
d'administrateur).

### Ouvrir le port

- Windows : `New-NetFirewallRule -DisplayName "Ocekour Sonde" -Direction Inbound -Protocol TCP -LocalPort 47120 -Action Allow`
- Linux (firewalld) : `firewall-cmd --permanent --add-port=47120/tcp && firewall-cmd --reload`

TLS est obligatoire dès que la sonde écoute ailleurs que sur `127.0.0.1`.

## 2. Se connecter (poste)

**+** → **Sonde**, puis :

1. **Adresse de la sonde** : `https://srv-recette-01:47120`.
2. **Empreinte du certificat** : collez celle reçue, ou **Lire sur la sonde** puis comparez-la
   avec celle de l'administrateur avant **Faire confiance à ce certificat**. Seul ce certificat
   sera accepté : une machine qui se ferait passer pour la sonde est refusée.
3. **Jeton de la sonde** : stocké chiffré sur le poste, jamais dans `config.yaml`.
4. **Tester la connexion**, puis **Enregistrer**.

Si l'administrateur fournit un certificat signé par l'autorité de l'entreprise, laissez
l'empreinte vide : le TLS habituel s'applique.

Les sources apparaissent dans l'arbre (badge **SONDE**), avec leur état (vert : lisible ; gris :
absente ; rouge : illisible), leur taille et leur âge. Une source de type motif se déplie sur
ses fichiers ; **▶ suivre** suit le plus récent, avec bascule à la rotation.

Un onglet de sonde a toutes les fonctions d'un onglet de logs : filtres, export, assemblage
avec des pods ou des topics, **Lien** de partage (sans jeton ni empreinte : votre collègue doit
avoir sa propre connexion à la sonde, proposée à la création sinon). Si le réseau coupe, le flux
reprend là où il s'était arrêté.

Les agents (MCP) voient les sondes dans `connexions_lister` et lisent leurs sources avec
`logs_lire` (chemin `[source]` ou `[source, fichier]`).

## 3. Exploitation

| Commande                                | Effet                                                                 |
| --------------------------------------- | --------------------------------------------------------------------- |
| `sonde statut`                          | Postes abonnés en ce moment (poste, IP, source, depuis quand, débit). |
| `sonde jeton renouveler`                | Nouveau jeton ; l'ancien est refusé **aussitôt**, flux en cours fermés. |
| `sonde verifier`                        | État de chaque source, empreinte du certificat.                       |

Le journal (`logs/main.log` à côté de `sonde.yaml`) trace démarrages, accès, refus et erreurs
de lecture — jamais le contenu des logs ni les jetons. Après 10 échecs d'authentification en
une minute, une adresse IP est bloquée 5 minutes.

**Mise à jour** : remplacez l'exécutable et redémarrez le service.

## Dépannage

| Message côté poste                                          | Cause et remède                                                                      |
| ----------------------------------------------------------- | ------------------------------------------------------------------------------------ |
| « Jeton de la sonde absent ou invalide »                    | Jeton renouvelé sur la sonde : demandez le nouveau, puis **Modifier** la connexion.  |
| « Jeton de la sonde renouvelé : mettez à jour la connexion » | Idem, pendant qu'un onglet était ouvert.                                            |
| « le certificat présenté (…) n'est pas celui qui a été approuvé » | Certificat régénéré… ou usurpation. Vérifiez l'empreinte auprès de l'administrateur avant d'approuver la nouvelle. |
| « Sonde saturée » / « Trop de flux ouverts depuis ce poste » | Limites `flux_simultanes` / `flux_par_poste` atteintes : fermez des onglets.         |
| « Trop d'échecs d'authentification »                        | 10 jetons faux en une minute depuis votre adresse : attendez 5 minutes.              |
| Connexion impossible                                        | Sonde arrêtée, port fermé (pare-feu), ou mauvaise adresse.                           |
