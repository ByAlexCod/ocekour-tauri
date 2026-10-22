# Rancher : logs des conteneurs

[← Sommaire](README.md)

## Créer la connexion

1. Récupérez un **token d'API** dans Rancher : avatar en haut à droite → **API & Keys** →
   **Add Key**. Laissez « Scope » vide pour accéder à tous vos clusters. Copiez le
   _Bearer Token_ (`token-xxxxx:yyyy…`).
2. Dans Ocekour : **+** → **Rancher**.
3. Saisissez l'**URL Rancher** (par ex. `https://rancher.intra`) et collez le **token**.
4. Choisissez l'option **Certificat** :
   - **Vérifier le certificat** (par défaut) : les autorités installées dans Windows sont
     reconnues. Si le certificat de votre entreprise n'est pas reconnu, ajoutez son
     autorité avec **Ajouter un certificat d'autorité (.cer)…**.
   - **Ignorer la vérification SSL** : à éviter, uniquement en dernier recours.
5. Cliquez sur **Tester et charger les clusters**, **cochez les clusters** qui vous
   intéressent, donnez un nom à la connexion et cliquez sur **Enregistrer**.

> Vous avez déjà un kubeconfig Rancher ? **Importer un kubeconfig…** remplit l'URL, le token
> et le cluster.

Pour modifier une connexion (clusters cochés, token expiré…), cliquez sur **Modifier** à côté
de son nom dans l'arbre.

## Parcourir

L'arbre descend ainsi : **connexion › cluster › namespace › pod › conteneur**.

- Chaque pod porte un point de santé : vert = OK, orange = pas encore prêt ou en attente,
  rouge = en échec (CrashLoopBackOff, OOM, Evicted…), gris = terminé. Son état et son nombre de
  redémarrages s'affichent au survol.
- Cliquez sur un **conteneur** pour ouvrir ses logs.

## Options du flux

| Option                     | Effet                                                                                                                  |
| -------------------------- | ---------------------------------------------------------------------------------------------------------------------- |
| **Suivi** / **Historique** | En direct, ou lecture ponctuelle puis arrêt                                                                            |
| **Lignes**                 | Nombre de dernières lignes chargées à l'ouverture (100, 1 000, 10 000 ou Tout)                                         |
| **Depuis**                 | Seulement les 5 dernières minutes, la dernière heure ou les dernières 24 h                                             |
| **Précédent**              | Logs de l'instance **précédente** du conteneur (après un redémarrage ou un crash). Grisé s'il n'y en a pas             |
| **⏸ Pause**                | Fige l'affichage. Les lignes continuent d'arriver en mémoire et un bouton « ↓ n nouvelles lignes » permet de reprendre |
| **Effacer**                | Vide l'affichage                                                                                                       |
| **⟳**                      | Relance le flux                                                                                                        |

## Ce qu'Ocekour gère pour vous

- **Coupure réseau** : reconnexion automatique, **sans doublon ni trou** (la lecture reprend
  à l'horodatage de la dernière ligne reçue).
- **Redémarrage du conteneur** : une ligne « Conteneur redémarré » s'insère dans le flux.
- **Pod supprimé** (redéploiement) : le flux s'arrête et Ocekour propose des boutons
  **Ouvrir <nouveau pod>** pour les pods actuels du même workload.
- **Mémoire** : au-delà de 100 000 lignes par onglet (réglable), les plus anciennes sont
  retirées. Le compteur « n retirées » l'indique.
