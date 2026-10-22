import type { ServiceId } from '../../../shared/api';

/** Étapes de création du jeton, affichées sous chaque service. */
export const HELP: Record<ServiceId, { steps: string[]; note?: string }> = {
  gitlab: {
    steps: [
      'Cliquez sur « Créer un jeton ↗ » : la page GitLab s’ouvre avec le nom « Ocekour » et la portée « api » déjà remplis.',
      'Choisissez une date d’expiration (souvent imposée par l’instance), puis « Create personal access token ».',
      'Copiez le jeton affiché en haut de la page : GitLab ne le montre qu’une fois.',
      'Revenez ici : « Coller le jeton », puis « Enregistrer ». Ocekour le teste aussitôt.',
    ],
    note: 'La page ne s’ouvre pas (GitLab avant la version 16) ? Utilisez « ancienne page ».',
  },
  jira: {
    steps: [
      'Cliquez sur « Créer un jeton ↗ » : votre profil Jira s’ouvre sur « Jetons d’accès personnels ».',
      'Cliquez sur « Créer un jeton », nommez-le « Ocekour », choisissez une expiration, puis « Créer ».',
      'Copiez le jeton affiché (il n’est plus visible ensuite), puis collez-le ici.',
    ],
    note: 'Pas de menu « Jetons d’accès personnels » ? L’administrateur Jira les a désactivés.',
  },
  confluence: {
    steps: [
      'Cliquez sur « Créer un jeton ↗ » : la page « Jetons d’accès personnels » de votre profil Confluence s’ouvre.',
      'Cliquez sur « Créer un jeton », nommez-le « Ocekour », choisissez une expiration, puis « Créer ».',
      'Copiez le jeton affiché (il n’est plus visible ensuite), puis collez-le ici.',
    ],
  },
  rocketchat: {
    steps: [
      'Cliquez sur « Créer un jeton ↗ » : Mon compte → Jetons d’accès personnels s’ouvre dans Rocket.Chat.',
      'Nommez le jeton « Ocekour » ; ne cochez pas « Ignorer l’authentification à deux facteurs » ; cliquez sur « Ajouter ».',
      'Rocket.Chat affiche deux valeurs : le jeton et votre identifiant utilisateur (« Your user Id »). Copiez les deux ici.',
    ],
    note: 'Pas de section « Jetons d’accès personnels » ? Demandez à l’administrateur la permission « create-personal-access-tokens ».',
  },
};

export const PLACEHOLDER: Record<ServiceId, string> = {
  gitlab: 'https://gitlab.exemple.fr',
  jira: 'https://jira.exemple.fr',
  confluence: 'https://confluence.exemple.fr',
  rocketchat: 'https://chat.exemple.fr',
};

export const WRITE_HINT: Record<ServiceId, string> = {
  gitlab: 'commentaires, issues, MR, relances de pipeline',
  jira: 'tickets, commentaires, statuts, temps passé',
  confluence: 'pages et commentaires',
  rocketchat: 'messages et réactions',
};

/** URL, TLS ou CA modifiés par rapport au compte enregistré. */
export function accountDirty(
  account: { url?: string; tls: { insecure: boolean; caFile?: string } },
  form: { url: string; insecure: boolean; caFile?: string },
): boolean {
  return (
    form.url.trim().replace(/\/+$/, '') !== (account.url ?? '') ||
    form.insecure !== account.tls.insecure ||
    form.caFile !== account.tls.caFile
  );
}
