//! Anonymous pseudo + colour generator. Mirrors the web app's
//! `pseudoGenerator.js`: random pick from a French list of animals /
//! fruits / vegetables, plus a vibrant HSL colour. Generated once per
//! session and reused for every chat message until app restart.

use std::sync::OnceLock;

const ANIMALS: &[&str] = &[
    // Mammifères
    "Lion", "Tigre", "Ours", "Loup", "Renard", "Cerf", "Biche", "Sanglier", "Lynx", "Puma",
    "Jaguar", "Panthère", "Guépard", "Léopard", "Hyène", "Chacal", "Coyote", "Blaireau",
    "Loutre", "Castor", "Raton", "Moufette", "Belette", "Hermine", "Martre", "Furet", "Vison",
    "Putois", "Hérisson", "Taupe", "Écureuil", "Marmotte", "Hamster", "Chinchilla", "Capybara",
    "Porc-épic", "Lapin", "Lièvre", "Koala", "Kangourou", "Wallaby", "Wombat", "Opossum",
    "Tatou", "Paresseux", "Fourmilier", "Pangolin", "Suricate", "Mangouste", "Genette",
    "Civette", "Gorille", "Chimpanzé", "Orang-outan", "Gibbon", "Babouin", "Mandrill",
    "Macaque", "Lémurien", "Tarsier", "Éléphant", "Rhinocéros", "Hippopotame", "Girafe",
    "Zèbre", "Bison", "Buffle", "Yak", "Gnou", "Antilope", "Gazelle", "Impala", "Oryx",
    "Chamois", "Bouquetin", "Mouflon", "Chameau", "Dromadaire", "Lama", "Alpaga", "Morse",
    "Phoque", "Otarie", "Lamantin", "Dugong", "Dauphin", "Orque", "Baleine", "Narval",
    "Béluga", "Chauve-souris", "Fennec", "Panda", "Raton-laveur", "Coati", "Kinkajou",
    "Okapi", "Tapir", "Pécari", "Dingo",
    // Oiseaux
    "Aigle", "Faucon", "Buse", "Vautour", "Condor", "Hibou", "Chouette", "Harfang", "Milan",
    "Épervier", "Autour", "Pélican", "Héron", "Cigogne", "Flamant", "Ibis", "Spatule",
    "Grue", "Albatros", "Mouette", "Goéland", "Sterne", "Macareux", "Pingouin", "Manchot",
    "Fou", "Cormoran", "Frégate", "Martin-pêcheur", "Guêpier", "Huppe", "Calao", "Toucan",
    "Perroquet", "Ara", "Cacatoès", "Perruche", "Inséparable", "Colibri", "Martinet",
    "Hirondelle", "Moineau", "Mésange", "Rouge-gorge", "Merle", "Grive", "Rossignol",
    "Fauvette", "Pinson", "Chardonneret", "Bouvreuil", "Loriot", "Étourneau", "Corbeau",
    "Corneille", "Pie", "Geai", "Choucas", "Crave", "Jaseur", "Pigeon", "Tourterelle",
    "Coucou", "Caille", "Perdrix", "Faisan", "Dindon", "Paon", "Casoar", "Émeu", "Autruche",
    "Kiwi", "Nandou", "Outarde", "Bernache", "Canard", "Cygne", "Oie", "Sarcelle", "Harle",
    // Reptiles
    "Crocodile", "Alligator", "Caïman", "Gavial", "Varan", "Iguane", "Gecko", "Caméléon",
    "Basilic", "Dragon", "Cobra", "Mamba", "Vipère", "Python", "Anaconda", "Boa",
    "Couleuvre", "Tortue",
    // Amphibiens
    "Grenouille", "Crapaud", "Salamandre", "Triton", "Axolotl",
    // Poissons
    "Requin", "Raie", "Espadon", "Marlin", "Thon", "Barracuda", "Piranha", "Poisson-clown",
    "Poisson-lune", "Hippocampe", "Murène", "Anguille", "Mérou", "Perche", "Brochet",
    "Carpe", "Truite", "Saumon", "Esturgeon",
    // Invertébrés
    "Pieuvre", "Calmar", "Seiche", "Nautile", "Homard", "Crabe", "Crevette", "Scorpion",
    "Mante", "Scarabée", "Coccinelle", "Libellule", "Papillon", "Luciole", "Cigale",
    "Criquet", "Sauterelle", "Fourmi", "Abeille", "Guêpe", "Frelon", "Araignée", "Méduse",
    "Étoile-de-mer", "Oursin", "Corail", "Poulpe",
];

const FRUITS: &[&str] = &[
    "Abricot", "Açaï", "Airelle", "Amande", "Ananas", "Arbouse", "Avocat", "Banane",
    "Bergamote", "Cacao", "Canneberge", "Carambole", "Cassis", "Cerise", "Châtaigne",
    "Citron", "Clémentine", "Coco", "Coing", "Combava", "Cranberry", "Datte", "Durian",
    "Figue", "Fraise", "Framboise", "Fruit-de-la-passion", "Goyave", "Grenade",
    "Grenadille", "Groseille", "Jacquier", "Jujube", "Kaki", "Kiwi", "Kumquat", "Litchi",
    "Longane", "Mandarine", "Mangoustan", "Mangue", "Melon", "Mirabelle", "Mûre",
    "Myrtille", "Nectarine", "Nèfle", "Noisette", "Noix", "Olive", "Orange", "Pamplemousse",
    "Papaye", "Pastèque", "Pêche", "Physalis", "Pistache", "Pitaya", "Poire", "Pomelo",
    "Pomme", "Prune", "Quetsche", "Raisin", "Ramboutan", "Rhubarbe", "Sapotille", "Tamarin",
    "Tangerine", "Yuzu",
];

const LEGUMES: &[&str] = &[
    "Ail", "Artichaut", "Asperge", "Aubergine", "Betterave", "Brocoli", "Butternut",
    "Carotte", "Céleri", "Cerfeuil", "Champignon", "Chou", "Chou-fleur", "Ciboulette",
    "Citrouille", "Concombre", "Cornichon", "Courge", "Courgette", "Cresson", "Échalote",
    "Endive", "Épinard", "Estragon", "Fenouil", "Fève", "Gingembre", "Haricot", "Laitue",
    "Lentille", "Mâche", "Maïs", "Manioc", "Navet", "Oignon", "Oseille", "Panais", "Patate",
    "Pâtisson", "Persil", "Petit-pois", "Piment", "Poireau", "Poivron", "Potiron",
    "Potimarron", "Radis", "Roquette", "Rutabaga", "Salsifis", "Shiso", "Soja", "Taro",
    "Tomate", "Topinambour", "Truffe", "Wasabi",
];

fn rand_u32() -> u32 {
    // Tiny seedless RNG using nanoseconds — good enough for picking
    // a name + colour. No need for a crypto-grade PRNG here.
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    // xorshift to spread bits a bit
    let mut x = nanos.wrapping_mul(0x9E37_79B1).wrapping_add(0xDEAD_BEEF);
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    x
}

fn pick(arr: &[&str]) -> String {
    let i = (rand_u32() as usize) % arr.len();
    arr[i].to_string()
}

pub fn generate_pseudo() -> String {
    // Pick from one of three pools, weighted by length so animals
    // (largest pool) dominate just like the web app's `[...A,...F,...L]`
    // concatenation does.
    let r = (rand_u32() as usize) % (ANIMALS.len() + FRUITS.len() + LEGUMES.len());
    if r < ANIMALS.len() {
        pick(ANIMALS)
    } else if r < ANIMALS.len() + FRUITS.len() {
        pick(FRUITS)
    } else {
        pick(LEGUMES)
    }
}

/// Twitch's canonical default chat colour palette — 15 vibrant colours,
/// same list used server-side and on the web. Picking from a fixed
/// palette keeps the chat coherent (recognisable Twitch-look) instead
/// of a random pastel HSL soup.
const TWITCH_COLORS: &[&str] = &[
    "#FF0000", "#0000FF", "#008000", "#B22222", "#FF7F50",
    "#9ACD32", "#FF4500", "#2E8B57", "#DAA520", "#D2691E",
    "#5F9EA0", "#1E90FF", "#FF69B4", "#8A2BE2", "#00FF7F",
];

pub fn generate_color() -> String {
    let i = (rand_u32() as usize) % TWITCH_COLORS.len();
    TWITCH_COLORS[i].to_string()
}

static SESSION_PSEUDO: OnceLock<String> = OnceLock::new();
static SESSION_COLOR: OnceLock<String> = OnceLock::new();

/// Stable per-session anonymous pseudo. First call generates, next
/// calls return the same value until app restart.
pub fn get_or_create_pseudo() -> String {
    SESSION_PSEUDO.get_or_init(generate_pseudo).clone()
}

pub fn get_or_create_color() -> String {
    SESSION_COLOR.get_or_init(generate_color).clone()
}
