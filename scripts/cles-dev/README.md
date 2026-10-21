# Clés de développement

`dev.pem` signe des clés d'activation **de développement**, acceptées uniquement par les
builds de développement et les tests, jamais par l'exe diffusé (build `OCEKOUR_RELEASE=1` en
CI). Elle est volontairement publique. Exemple :

    cargo run -p ocekour-headless -- cle generer --kid dev --label "Dev" --mode none
