// --- Animals ---
const ANIMALS = [
  // Mammifères
  'Lion', 'Tigre', 'Ours', 'Loup', 'Renard', 'Cerf', 'Biche', 'Sanglier', 'Lynx', 'Puma',
  'Jaguar', 'Panthère', 'Guépard', 'Léopard', 'Hyène', 'Chacal', 'Coyote', 'Blaireau', 'Loutre', 'Castor',
  'Raton', 'Moufette', 'Belette', 'Hermine', 'Martre', 'Furet', 'Vison', 'Putois', 'Hérisson', 'Taupe',
  'Écureuil', 'Marmotte', 'Hamster', 'Chinchilla', 'Capybara', 'Porc-épic', 'Lapin', 'Lièvre', 'Koala', 'Kangourou',
  'Wallaby', 'Wombat', 'Opossum', 'Tatou', 'Paresseux', 'Fourmilier', 'Pangolin', 'Suricate', 'Mangouste', 'Genette',
  'Civette', 'Gorille', 'Chimpanzé', 'Orang-outan', 'Gibbon', 'Babouin', 'Mandrill', 'Macaque', 'Lémurien', 'Tarsier',
  'Éléphant', 'Rhinocéros', 'Hippopotame', 'Girafe', 'Zèbre', 'Bison', 'Buffle', 'Yak', 'Gnou', 'Antilope',
  'Gazelle', 'Impala', 'Oryx', 'Chamois', 'Bouquetin', 'Mouflon', 'Chameau', 'Dromadaire', 'Lama', 'Alpaga',
  'Morse', 'Phoque', 'Otarie', 'Lamantin', 'Dugong', 'Dauphin', 'Orque', 'Baleine', 'Narval', 'Béluga',
  'Chauve-souris', 'Fennec', 'Panda', 'Raton-laveur', 'Coati', 'Kinkajou', 'Okapi', 'Tapir', 'Pécari', 'Dingo',
  // Oiseaux
  'Aigle', 'Faucon', 'Buse', 'Vautour', 'Condor', 'Hibou', 'Chouette', 'Harfang', 'Milan', 'Épervier',
  'Autour', 'Pélican', 'Héron', 'Cigogne', 'Flamant', 'Ibis', 'Spatule', 'Grue', 'Albatros', 'Mouette',
  'Goéland', 'Sterne', 'Macareux', 'Pingouin', 'Manchot', 'Fou', 'Cormoran', 'Frégate', 'Martin-pêcheur', 'Guêpier',
  'Huppe', 'Calao', 'Toucan', 'Perroquet', 'Ara', 'Cacatoès', 'Perruche', 'Inséparable', 'Colibri', 'Martinet',
  'Hirondelle', 'Moineau', 'Mésange', 'Rouge-gorge', 'Merle', 'Grive', 'Rossignol', 'Fauvette', 'Pinson', 'Chardonneret',
  'Bouvreuil', 'Loriot', 'Étourneau', 'Corbeau', 'Corneille', 'Pie', 'Geai', 'Choucas', 'Crave', 'Jaseur',
  'Pigeon', 'Tourterelle', 'Coucou', 'Caille', 'Perdrix', 'Faisan', 'Dindon', 'Paon', 'Casoar', 'Émeu',
  'Autruche', 'Kiwi', 'Nandou', 'Outarde', 'Bernache', 'Canard', 'Cygne', 'Oie', 'Sarcelle', 'Harle',
  // Reptiles
  'Crocodile', 'Alligator', 'Caïman', 'Gavial', 'Varan', 'Iguane', 'Gecko', 'Caméléon', 'Basilic', 'Dragon',
  'Cobra', 'Mamba', 'Vipère', 'Python', 'Anaconda', 'Boa', 'Couleuvre', 'Tortue',
  // Amphibiens
  'Grenouille', 'Crapaud', 'Salamandre', 'Triton', 'Axolotl',
  // Poissons
  'Requin', 'Raie', 'Espadon', 'Marlin', 'Thon', 'Barracuda', 'Piranha', 'Poisson-clown', 'Poisson-lune',
  'Hippocampe', 'Murène', 'Anguille', 'Mérou', 'Perche', 'Brochet', 'Carpe', 'Truite', 'Saumon', 'Esturgeon',
  // Invertébrés
  'Pieuvre', 'Calmar', 'Seiche', 'Nautile', 'Homard', 'Crabe', 'Crevette', 'Scorpion', 'Mante', 'Scarabée',
  'Coccinelle', 'Libellule', 'Papillon', 'Luciole', 'Cigale', 'Criquet', 'Sauterelle', 'Fourmi', 'Abeille', 'Guêpe',
  'Frelon', 'Araignée', 'Méduse', 'Étoile-de-mer', 'Oursin', 'Corail', 'Poulpe',
];

// --- Fruits ---
const FRUITS = [
  'Abricot', 'Açaï', 'Airelle', 'Amande', 'Ananas', 'Arbouse', 'Avocat', 'Banane', 'Bergamote', 'Cacao',
  'Canneberge', 'Carambole', 'Cassis', 'Cerise', 'Châtaigne', 'Citron', 'Clémentine', 'Coco', 'Coing', 'Combava',
  'Cranberry', 'Datte', 'Durian', 'Figue', 'Fraise', 'Framboise', 'Fruit-de-la-passion', 'Goyave', 'Grenade', 'Grenadille',
  'Groseille', 'Jacquier', 'Jujube', 'Kaki', 'Kiwi', 'Kumquat', 'Litchi', 'Longane', 'Mandarine', 'Mangoustan',
  'Mangue', 'Melon', 'Mirabelle', 'Mûre', 'Myrtille', 'Nectarine', 'Nèfle', 'Noisette', 'Noix', 'Olive',
  'Orange', 'Pamplemousse', 'Papaye', 'Pastèque', 'Pêche', 'Physalis', 'Pistache', 'Pitaya', 'Poire', 'Pomelo',
  'Pomme', 'Prune', 'Quetsche', 'Raisin', 'Ramboutan', 'Rhubarbe', 'Sapotille', 'Tamarin', 'Tangerine', 'Yuzu',
];

// --- Légumes ---
const LEGUMES = [
  'Ail', 'Artichaut', 'Asperge', 'Aubergine', 'Betterave', 'Brocoli', 'Butternut', 'Carotte', 'Céleri', 'Cerfeuil',
  'Champignon', 'Chou', 'Chou-fleur', 'Ciboulette', 'Citrouille', 'Concombre', 'Cornichon', 'Courge', 'Courgette', 'Cresson',
  'Échalote', 'Endive', 'Épinard', 'Estragon', 'Fenouil', 'Fève', 'Gingembre', 'Haricot', 'Laitue', 'Lentille',
  'Mâche', 'Maïs', 'Manioc', 'Navet', 'Oignon', 'Oseille', 'Panais', 'Patate', 'Pâtisson', 'Persil',
  'Petit-pois', 'Piment', 'Poireau', 'Poivron', 'Potiron', 'Potimarron', 'Radis', 'Roquette', 'Rutabaga', 'Salsifis',
  'Shiso', 'Soja', 'Taro', 'Tomate', 'Topinambour', 'Truffe', 'Wasabi',
];

const ALL_NAMES = [...ANIMALS, ...FRUITS, ...LEGUMES];

// --- Random color generator (vibrant, readable on dark background) ---
function randomColor() {
  const hue = Math.floor(Math.random() * 360);
  const saturation = 60 + Math.floor(Math.random() * 30); // 60-90%
  const lightness = 55 + Math.floor(Math.random() * 20);  // 55-75%
  return `hsl(${hue}, ${saturation}%, ${lightness}%)`;
}

function randomPick(arr) {
  return arr[Math.floor(Math.random() * arr.length)];
}

export function generatePseudo() {
  return randomPick(ALL_NAMES);
}

export function generateColor() {
  return randomColor();
}

export function getOrCreatePseudo() {
  let pseudo = sessionStorage.getItem('anonymousPseudo');
  if (!pseudo) {
    pseudo = generatePseudo();
    sessionStorage.setItem('anonymousPseudo', pseudo);
  }
  return pseudo;
}

export function getOrCreateColor() {
  let color = sessionStorage.getItem('anonymousColor');
  if (!color) {
    color = generateColor();
    sessionStorage.setItem('anonymousColor', color);
  }
  return color;
}
