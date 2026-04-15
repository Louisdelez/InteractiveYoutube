// Server-side pseudo generator — used as a *fallback* when a client connects
// and starts chatting before its `chat:setAnonymousName` packet has arrived
// (race conditions on connect, or clients that don't bother sending one).
// We never want to show "Anonyme-xxxx" — just always a French animal/fruit/
// vegetable, same vibe as the client/desktop generators.

const ANIMALS = [
  'Lion', 'Tigre', 'Ours', 'Loup', 'Renard', 'Cerf', 'Biche', 'Sanglier', 'Lynx', 'Puma',
  'Jaguar', 'Panthère', 'Guépard', 'Léopard', 'Hyène', 'Chacal', 'Coyote', 'Blaireau', 'Loutre', 'Castor',
  'Raton', 'Moufette', 'Belette', 'Hermine', 'Martre', 'Furet', 'Vison', 'Putois', 'Hérisson', 'Taupe',
  'Écureuil', 'Marmotte', 'Hamster', 'Chinchilla', 'Capybara', 'Lapin', 'Lièvre', 'Koala', 'Kangourou',
  'Wallaby', 'Wombat', 'Opossum', 'Tatou', 'Paresseux', 'Pangolin', 'Suricate', 'Mangouste',
  'Gorille', 'Chimpanzé', 'Gibbon', 'Babouin', 'Mandrill', 'Macaque', 'Lémurien',
  'Éléphant', 'Rhinocéros', 'Hippopotame', 'Girafe', 'Zèbre', 'Bison', 'Buffle', 'Yak', 'Gnou', 'Antilope',
  'Gazelle', 'Impala', 'Oryx', 'Chamois', 'Bouquetin', 'Mouflon', 'Chameau', 'Dromadaire', 'Lama', 'Alpaga',
  'Morse', 'Phoque', 'Otarie', 'Lamantin', 'Dauphin', 'Orque', 'Baleine', 'Narval', 'Béluga',
  'Fennec', 'Panda', 'Coati', 'Kinkajou', 'Okapi', 'Tapir', 'Pécari', 'Dingo',
  'Aigle', 'Faucon', 'Buse', 'Vautour', 'Condor', 'Hibou', 'Chouette', 'Harfang', 'Milan', 'Épervier',
  'Pélican', 'Héron', 'Cigogne', 'Flamant', 'Ibis', 'Spatule', 'Grue', 'Albatros', 'Mouette',
  'Goéland', 'Sterne', 'Macareux', 'Pingouin', 'Manchot', 'Cormoran', 'Toucan', 'Perroquet',
  'Ara', 'Cacatoès', 'Perruche', 'Colibri', 'Hirondelle', 'Moineau', 'Mésange', 'Merle', 'Grive',
  'Rossignol', 'Pinson', 'Chardonneret', 'Bouvreuil', 'Étourneau', 'Corbeau', 'Pie', 'Geai',
  'Pigeon', 'Tourterelle', 'Caille', 'Perdrix', 'Faisan', 'Dindon', 'Paon', 'Émeu',
  'Autruche', 'Kiwi', 'Canard', 'Cygne', 'Oie',
  'Crocodile', 'Alligator', 'Caïman', 'Varan', 'Iguane', 'Gecko', 'Caméléon',
  'Cobra', 'Mamba', 'Vipère', 'Python', 'Anaconda', 'Boa', 'Couleuvre', 'Tortue',
  'Grenouille', 'Crapaud', 'Salamandre', 'Triton', 'Axolotl',
  'Requin', 'Raie', 'Espadon', 'Marlin', 'Thon', 'Barracuda', 'Piranha',
  'Hippocampe', 'Murène', 'Anguille', 'Mérou', 'Brochet', 'Carpe', 'Truite', 'Saumon',
  'Pieuvre', 'Calmar', 'Seiche', 'Homard', 'Crabe', 'Crevette', 'Scorpion', 'Mante', 'Scarabée',
  'Coccinelle', 'Libellule', 'Papillon', 'Luciole', 'Cigale', 'Sauterelle', 'Fourmi', 'Abeille',
  'Frelon', 'Araignée', 'Méduse',
];

const FRUITS = [
  'Abricot', 'Açaï', 'Airelle', 'Amande', 'Ananas', 'Arbouse', 'Avocat', 'Banane', 'Bergamote', 'Cacao',
  'Canneberge', 'Carambole', 'Cassis', 'Cerise', 'Châtaigne', 'Citron', 'Clémentine', 'Coco', 'Coing',
  'Datte', 'Durian', 'Figue', 'Fraise', 'Framboise', 'Goyave', 'Grenade',
  'Groseille', 'Jacquier', 'Jujube', 'Kaki', 'Kumquat', 'Litchi', 'Mandarine',
  'Mangue', 'Melon', 'Mirabelle', 'Mûre', 'Myrtille', 'Nectarine', 'Nèfle', 'Noisette', 'Noix', 'Olive',
  'Orange', 'Pamplemousse', 'Papaye', 'Pastèque', 'Pêche', 'Physalis', 'Pistache', 'Poire',
  'Pomme', 'Prune', 'Quetsche', 'Raisin', 'Ramboutan', 'Rhubarbe', 'Tamarin', 'Yuzu',
];

const LEGUMES = [
  'Ail', 'Artichaut', 'Asperge', 'Aubergine', 'Betterave', 'Brocoli', 'Butternut', 'Carotte', 'Céleri',
  'Champignon', 'Chou', 'Chou-fleur', 'Citrouille', 'Concombre', 'Courge', 'Courgette', 'Cresson',
  'Échalote', 'Endive', 'Épinard', 'Fenouil', 'Fève', 'Gingembre', 'Haricot', 'Laitue', 'Lentille',
  'Mâche', 'Maïs', 'Manioc', 'Navet', 'Oignon', 'Panais', 'Patate', 'Persil',
  'Petit-pois', 'Piment', 'Poireau', 'Poivron', 'Potiron', 'Potimarron', 'Radis', 'Roquette', 'Salsifis',
  'Soja', 'Tomate', 'Topinambour', 'Truffe',
];

const ALL_NAMES = [...ANIMALS, ...FRUITS, ...LEGUMES];

function randomPick(arr) {
  return arr[Math.floor(Math.random() * arr.length)];
}

function generatePseudo() {
  return randomPick(ALL_NAMES);
}

// Twitch's canonical default chat colour palette (15 colours). Same
// list used everywhere — server fallback, registered users (db.js),
// web (pseudoGenerator.js), desktop (pseudo.rs).
const TWITCH_COLORS = [
  '#FF0000', '#0000FF', '#008000', '#B22222', '#FF7F50',
  '#9ACD32', '#FF4500', '#2E8B57', '#DAA520', '#D2691E',
  '#5F9EA0', '#1E90FF', '#FF69B4', '#8A2BE2', '#00FF7F',
];

function generateColor() {
  return randomPick(TWITCH_COLORS);
}

module.exports.TWITCH_COLORS = TWITCH_COLORS;

module.exports = { generatePseudo, generateColor };
