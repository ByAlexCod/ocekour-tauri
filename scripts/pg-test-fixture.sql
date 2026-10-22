-- Base PostgreSQL des tests (BDD_TEST_PG) : utilisateur lecteur/lecteur, table et vue.
CREATE TABLE clients (id integer PRIMARY KEY, nom text, cree timestamptz DEFAULT now());
INSERT INTO clients (id, nom) VALUES (1, 'Alice'), (2, 'Bob');
CREATE VIEW v_clients AS SELECT id, nom FROM clients;
CREATE ROLE lecteur LOGIN PASSWORD 'lecteur';
GRANT SELECT ON clients, v_clients TO lecteur;
