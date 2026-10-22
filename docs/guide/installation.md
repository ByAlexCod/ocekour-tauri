# Installation et premier lancement

[← Sommaire](README.md)

## Installer

Il n'y a **rien à installer**. Copiez `Ocekour-<version>.exe` où vous voulez (Bureau,
`C:\Outils\`…) et double-cliquez dessus.

- Windows 10 ou 11, 64 bits.
- Aucun droit administrateur requis.
- L'exe (une vingtaine de Mo) démarre **immédiatement**, sans rien décompresser : l'affichage
  utilise **Microsoft Edge WebView2**, déjà présent sur Windows 10 et 11.
- Pour un poste sans WebView2, l'installateur `Ocekour_<version>_x64-setup.exe` (sans droit
  administrateur) l'installe au besoin.
- `ocekour-headless.exe`, livré à côté, offre les modes sans interface (`--mcp`, `--mcp-http`,
  `--notify`) sans dépendre de WebView2 ; `Ocekour-<version>.exe --mcp` fonctionne aussi.

### Avertissement SmartScreen

L'exe n'est pas signé. Au premier lancement, Windows peut afficher « Windows a protégé
votre ordinateur ». Cliquez sur **Informations complémentaires**, puis sur
**Exécuter quand même**. Cela n'est demandé qu'une fois par version.

### Activation

Au premier lancement, Ocekour demande une **clé d'activation**, qui commence par
`OCEKOUR1-`. Elle vous est remise par l'administrateur d'Ocekour de votre équipe.
Collez-la puis cliquez sur **Activer**. La clé est enregistrée chiffrée sur le poste et n'est
plus redemandée.

Si votre équipe l'a prévu, la clé **préconfigure vos sources** : les connexions Rancher,
Kafka ou S3 de l'équipe apparaissent dans l'arbre dès l'activation. Il ne reste qu'à saisir
**vos** identifiants : cliquez sur **Mettre à jour les identifiants** sous chaque connexion.
La clé ne contient aucun secret. Vos connexions existantes sont conservées.

Selon son type, la clé détermine aussi l'envoi de [statistiques d'utilisation
anonymes](configuration.md#statistiques-dutilisation) :

| Type de clé       | Effet                                                                                                                                                                          |
| ----------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Sans suivi        | Aucune statistique                                                                                                                                                             |
| Suivi si possible | Statistiques envoyées quand le serveur de l'équipe est joignable                                                                                                               |
| Suivi obligatoire | Ocekour doit joindre le serveur de statistiques de l'équipe pour fonctionner. S'il est injoignable (hors réseau, VPN coupé…), un écran l'indique, avec un bouton **Réessayer** |

Le type de la clé et son libellé sont visibles dans **Paramètres → Licence**, qui propose
aussi **Changer de clé**.

### Une seule instance

Relancer l'exe alors qu'Ocekour est déjà ouvert ramène simplement la fenêtre existante au
premier plan.

## Où sont mes données ?

Tout est dans `%APPDATA%\Ocekour\` (collez ce chemin dans l'Explorateur Windows) :

| Fichier         | Contenu                                                                                                                                         |
| --------------- | ----------------------------------------------------------------------------------------------------------------------------------------------- |
| `config.yaml`   | Connexions, profils, favoris, préférences. Ne contient **aucun secret**. Modifiable à la main ou depuis **Paramètres → config.yaml**            |
| `secrets.json`  | Tokens, mots de passe et clés secrètes, **chiffrés** (AES-256-GCM) par la clé locale ci-dessous. Jamais en clair                                |
| `secret.key`    | Clé de chiffrement des secrets, propre au poste. À protéger comme les secrets ; ne pas la copier ailleurs. Supprimée = secrets à ressaisir      |
| `logs\main.log` | Journal d'Ocekour : démarrage, connexions, flux, erreurs, opérations S3. Sans aucun secret. Accessible par **Paramètres → Afficher le journal** |

> Recopier `config.yaml` sur un autre poste reprend vos connexions et profils. En revanche, les secrets
> ne suivent pas : ressaisissez-les une fois par connexion (**Modifier** sur la connexion).

## Mettre à jour

Remplacez l'ancien exe par le nouveau. La configuration et les secrets, qui sont dans
`%APPDATA%\Ocekour\`, sont conservés.

## Désinstaller

1. Supprimez l'exe.
2. Si vous voulez aussi effacer la configuration et les secrets, supprimez le dossier `%APPDATA%\Ocekour\`.
