//! The arXiv subject taxonomy, transcribed from <https://arxiv.org/category_taxonomy>.
//!
//! arXiv has no API for this, so the tree is baked in. Groups (e.g. "Physics")
//! contain archives (e.g. "Astrophysics"), which contain the categories that
//! `cat:` queries against the arXiv API actually accept.

/// A leaf subject, e.g. `cs.AI`.
pub struct Category {
    pub id: &'static str,
    pub name: &'static str,
}

/// A collection of categories. Most groups have a single archive mirroring the
/// group itself; Physics is the one that genuinely splits.
pub struct Archive {
    pub id: &'static str,
    pub name: &'static str,
    pub categories: &'static [Category],
}

/// A top-level subject group, as listed on the arXiv landing page.
pub struct Group {
    pub name: &'static str,
    pub archives: &'static [Archive],
}

/// Look up a category by id, returning it alongside its owning group.
pub fn find(id: &str) -> Option<(&'static Group, &'static Category)> {
    GROUPS.iter().find_map(|g| {
        g.archives
            .iter()
            .flat_map(|a| a.categories.iter())
            .find(|c| c.id == id)
            .map(|c| (g, c))
    })
}

/// Total number of categories in the taxonomy.
pub fn category_count() -> usize {
    GROUPS
        .iter()
        .flat_map(|g| g.archives.iter())
        .map(|a| a.categories.len())
        .sum()
}

pub static GROUPS: &[Group] = &[
    Group {
        name: "Computer Science",
        archives: &[
            Archive {
                id: "cs",
                name: "Computer Science",
                categories: &[
                    Category { id: "cs.AI", name: "Artificial Intelligence" },
                    Category { id: "cs.AR", name: "Hardware Architecture" },
                    Category { id: "cs.CC", name: "Computational Complexity" },
                    Category { id: "cs.CE", name: "Computational Engineering, Finance, and Science" },
                    Category { id: "cs.CG", name: "Computational Geometry" },
                    Category { id: "cs.CL", name: "Computation and Language" },
                    Category { id: "cs.CR", name: "Cryptography and Security" },
                    Category { id: "cs.CV", name: "Computer Vision and Pattern Recognition" },
                    Category { id: "cs.CY", name: "Computers and Society" },
                    Category { id: "cs.DB", name: "Databases" },
                    Category { id: "cs.DC", name: "Distributed, Parallel, and Cluster Computing" },
                    Category { id: "cs.DL", name: "Digital Libraries" },
                    Category { id: "cs.DM", name: "Discrete Mathematics" },
                    Category { id: "cs.DS", name: "Data Structures and Algorithms" },
                    Category { id: "cs.ET", name: "Emerging Technologies" },
                    Category { id: "cs.FL", name: "Formal Languages and Automata Theory" },
                    Category { id: "cs.GL", name: "General Literature" },
                    Category { id: "cs.GR", name: "Graphics" },
                    Category { id: "cs.GT", name: "Computer Science and Game Theory" },
                    Category { id: "cs.HC", name: "Human-Computer Interaction" },
                    Category { id: "cs.IR", name: "Information Retrieval" },
                    Category { id: "cs.IT", name: "Information Theory" },
                    Category { id: "cs.LG", name: "Machine Learning" },
                    Category { id: "cs.LO", name: "Logic in Computer Science" },
                    Category { id: "cs.MA", name: "Multiagent Systems" },
                    Category { id: "cs.MM", name: "Multimedia" },
                    Category { id: "cs.MS", name: "Mathematical Software" },
                    Category { id: "cs.NA", name: "Numerical Analysis" },
                    Category { id: "cs.NE", name: "Neural and Evolutionary Computing" },
                    Category { id: "cs.NI", name: "Networking and Internet Architecture" },
                    Category { id: "cs.OH", name: "Other Computer Science" },
                    Category { id: "cs.OS", name: "Operating Systems" },
                    Category { id: "cs.PF", name: "Performance" },
                    Category { id: "cs.PL", name: "Programming Languages" },
                    Category { id: "cs.RO", name: "Robotics" },
                    Category { id: "cs.SC", name: "Symbolic Computation" },
                    Category { id: "cs.SD", name: "Sound" },
                    Category { id: "cs.SE", name: "Software Engineering" },
                    Category { id: "cs.SI", name: "Social and Information Networks" },
                    Category { id: "cs.SY", name: "Systems and Control" },
                ],
            },
        ],
    },
    Group {
        name: "Economics",
        archives: &[
            Archive {
                id: "econ",
                name: "Economics",
                categories: &[
                    Category { id: "econ.EM", name: "Econometrics" },
                    Category { id: "econ.GN", name: "General Economics" },
                    Category { id: "econ.TH", name: "Theoretical Economics" },
                ],
            },
        ],
    },
    Group {
        name: "Electrical Engineering and Systems Science",
        archives: &[
            Archive {
                id: "eess",
                name: "Electrical Engineering and Systems Science",
                categories: &[
                    Category { id: "eess.AS", name: "Audio and Speech Processing" },
                    Category { id: "eess.IV", name: "Image and Video Processing" },
                    Category { id: "eess.SP", name: "Signal Processing" },
                    Category { id: "eess.SY", name: "Systems and Control" },
                ],
            },
        ],
    },
    Group {
        name: "Mathematics",
        archives: &[
            Archive {
                id: "math",
                name: "Mathematics",
                categories: &[
                    Category { id: "math.AC", name: "Commutative Algebra" },
                    Category { id: "math.AG", name: "Algebraic Geometry" },
                    Category { id: "math.AP", name: "Analysis of PDEs" },
                    Category { id: "math.AT", name: "Algebraic Topology" },
                    Category { id: "math.CA", name: "Classical Analysis and ODEs" },
                    Category { id: "math.CO", name: "Combinatorics" },
                    Category { id: "math.CT", name: "Category Theory" },
                    Category { id: "math.CV", name: "Complex Variables" },
                    Category { id: "math.DG", name: "Differential Geometry" },
                    Category { id: "math.DS", name: "Dynamical Systems" },
                    Category { id: "math.FA", name: "Functional Analysis" },
                    Category { id: "math.GM", name: "General Mathematics" },
                    Category { id: "math.GN", name: "General Topology" },
                    Category { id: "math.GR", name: "Group Theory" },
                    Category { id: "math.GT", name: "Geometric Topology" },
                    Category { id: "math.HO", name: "History and Overview" },
                    Category { id: "math.IT", name: "Information Theory" },
                    Category { id: "math.KT", name: "K-Theory and Homology" },
                    Category { id: "math.LO", name: "Logic" },
                    Category { id: "math.MG", name: "Metric Geometry" },
                    Category { id: "math.MP", name: "Mathematical Physics" },
                    Category { id: "math.NA", name: "Numerical Analysis" },
                    Category { id: "math.NT", name: "Number Theory" },
                    Category { id: "math.OA", name: "Operator Algebras" },
                    Category { id: "math.OC", name: "Optimization and Control" },
                    Category { id: "math.PR", name: "Probability" },
                    Category { id: "math.QA", name: "Quantum Algebra" },
                    Category { id: "math.RA", name: "Rings and Algebras" },
                    Category { id: "math.RT", name: "Representation Theory" },
                    Category { id: "math.SG", name: "Symplectic Geometry" },
                    Category { id: "math.SP", name: "Spectral Theory" },
                    Category { id: "math.ST", name: "Statistics Theory" },
                ],
            },
        ],
    },
    Group {
        name: "Physics",
        archives: &[
            Archive {
                id: "astro-ph",
                name: "Astrophysics",
                categories: &[
                    Category { id: "astro-ph.CO", name: "Cosmology and Nongalactic Astrophysics" },
                    Category { id: "astro-ph.EP", name: "Earth and Planetary Astrophysics" },
                    Category { id: "astro-ph.GA", name: "Astrophysics of Galaxies" },
                    Category { id: "astro-ph.HE", name: "High Energy Astrophysical Phenomena" },
                    Category { id: "astro-ph.IM", name: "Instrumentation and Methods for Astrophysics" },
                    Category { id: "astro-ph.SR", name: "Solar and Stellar Astrophysics" },
                ],
            },
            Archive {
                id: "cond-mat",
                name: "Condensed Matter",
                categories: &[
                    Category { id: "cond-mat.dis-nn", name: "Disordered Systems and Neural Networks" },
                    Category { id: "cond-mat.mes-hall", name: "Mesoscale and Nanoscale Physics" },
                    Category { id: "cond-mat.mtrl-sci", name: "Materials Science" },
                    Category { id: "cond-mat.other", name: "Other Condensed Matter" },
                    Category { id: "cond-mat.quant-gas", name: "Quantum Gases" },
                    Category { id: "cond-mat.soft", name: "Soft Condensed Matter" },
                    Category { id: "cond-mat.stat-mech", name: "Statistical Mechanics" },
                    Category { id: "cond-mat.str-el", name: "Strongly Correlated Electrons" },
                    Category { id: "cond-mat.supr-con", name: "Superconductivity" },
                ],
            },
            Archive {
                id: "gr-qc",
                name: "General Relativity and Quantum Cosmology",
                categories: &[
                    Category { id: "gr-qc", name: "General Relativity and Quantum Cosmology" },
                ],
            },
            Archive {
                id: "hep-ex",
                name: "High Energy Physics - Experiment",
                categories: &[
                    Category { id: "hep-ex", name: "High Energy Physics - Experiment" },
                ],
            },
            Archive {
                id: "hep-lat",
                name: "High Energy Physics - Lattice",
                categories: &[
                    Category { id: "hep-lat", name: "High Energy Physics - Lattice" },
                ],
            },
            Archive {
                id: "hep-ph",
                name: "High Energy Physics - Phenomenology",
                categories: &[
                    Category { id: "hep-ph", name: "High Energy Physics - Phenomenology" },
                ],
            },
            Archive {
                id: "hep-th",
                name: "High Energy Physics - Theory",
                categories: &[
                    Category { id: "hep-th", name: "High Energy Physics - Theory" },
                ],
            },
            Archive {
                id: "math-ph",
                name: "Mathematical Physics",
                categories: &[
                    Category { id: "math-ph", name: "Mathematical Physics" },
                ],
            },
            Archive {
                id: "nlin",
                name: "Nonlinear Sciences",
                categories: &[
                    Category { id: "nlin.AO", name: "Adaptation and Self-Organizing Systems" },
                    Category { id: "nlin.CD", name: "Chaotic Dynamics" },
                    Category { id: "nlin.CG", name: "Cellular Automata and Lattice Gases" },
                    Category { id: "nlin.PS", name: "Pattern Formation and Solitons" },
                    Category { id: "nlin.SI", name: "Exactly Solvable and Integrable Systems" },
                ],
            },
            Archive {
                id: "nucl-ex",
                name: "Nuclear Experiment",
                categories: &[
                    Category { id: "nucl-ex", name: "Nuclear Experiment" },
                ],
            },
            Archive {
                id: "nucl-th",
                name: "Nuclear Theory",
                categories: &[
                    Category { id: "nucl-th", name: "Nuclear Theory" },
                ],
            },
            Archive {
                id: "physics",
                name: "Physics",
                categories: &[
                    Category { id: "physics.acc-ph", name: "Accelerator Physics" },
                    Category { id: "physics.ao-ph", name: "Atmospheric and Oceanic Physics" },
                    Category { id: "physics.app-ph", name: "Applied Physics" },
                    Category { id: "physics.atm-clus", name: "Atomic and Molecular Clusters" },
                    Category { id: "physics.atom-ph", name: "Atomic Physics" },
                    Category { id: "physics.bio-ph", name: "Biological Physics" },
                    Category { id: "physics.chem-ph", name: "Chemical Physics" },
                    Category { id: "physics.class-ph", name: "Classical Physics" },
                    Category { id: "physics.comp-ph", name: "Computational Physics" },
                    Category { id: "physics.data-an", name: "Data Analysis, Statistics and Probability" },
                    Category { id: "physics.ed-ph", name: "Physics Education" },
                    Category { id: "physics.flu-dyn", name: "Fluid Dynamics" },
                    Category { id: "physics.gen-ph", name: "General Physics" },
                    Category { id: "physics.geo-ph", name: "Geophysics" },
                    Category { id: "physics.hist-ph", name: "History and Philosophy of Physics" },
                    Category { id: "physics.ins-det", name: "Instrumentation and Detectors" },
                    Category { id: "physics.med-ph", name: "Medical Physics" },
                    Category { id: "physics.optics", name: "Optics" },
                    Category { id: "physics.plasm-ph", name: "Plasma Physics" },
                    Category { id: "physics.pop-ph", name: "Popular Physics" },
                    Category { id: "physics.soc-ph", name: "Physics and Society" },
                    Category { id: "physics.space-ph", name: "Space Physics" },
                ],
            },
            Archive {
                id: "quant-ph",
                name: "Quantum Physics",
                categories: &[
                    Category { id: "quant-ph", name: "Quantum Physics" },
                ],
            },
        ],
    },
    Group {
        name: "Quantitative Biology",
        archives: &[
            Archive {
                id: "q-bio",
                name: "Quantitative Biology",
                categories: &[
                    Category { id: "q-bio.BM", name: "Biomolecules" },
                    Category { id: "q-bio.CB", name: "Cell Behavior" },
                    Category { id: "q-bio.GN", name: "Genomics" },
                    Category { id: "q-bio.MN", name: "Molecular Networks" },
                    Category { id: "q-bio.NC", name: "Neurons and Cognition" },
                    Category { id: "q-bio.OT", name: "Other Quantitative Biology" },
                    Category { id: "q-bio.PE", name: "Populations and Evolution" },
                    Category { id: "q-bio.QM", name: "Quantitative Methods" },
                    Category { id: "q-bio.SC", name: "Subcellular Processes" },
                    Category { id: "q-bio.TO", name: "Tissues and Organs" },
                ],
            },
        ],
    },
    Group {
        name: "Quantitative Finance",
        archives: &[
            Archive {
                id: "q-fin",
                name: "Quantitative Finance",
                categories: &[
                    Category { id: "q-fin.CP", name: "Computational Finance" },
                    Category { id: "q-fin.EC", name: "Economics" },
                    Category { id: "q-fin.GN", name: "General Finance" },
                    Category { id: "q-fin.MF", name: "Mathematical Finance" },
                    Category { id: "q-fin.PM", name: "Portfolio Management" },
                    Category { id: "q-fin.PR", name: "Pricing of Securities" },
                    Category { id: "q-fin.RM", name: "Risk Management" },
                    Category { id: "q-fin.ST", name: "Statistical Finance" },
                    Category { id: "q-fin.TR", name: "Trading and Market Microstructure" },
                ],
            },
        ],
    },
    Group {
        name: "Statistics",
        archives: &[
            Archive {
                id: "stat",
                name: "Statistics",
                categories: &[
                    Category { id: "stat.AP", name: "Applications" },
                    Category { id: "stat.CO", name: "Computation" },
                    Category { id: "stat.ME", name: "Methodology" },
                    Category { id: "stat.ML", name: "Machine Learning" },
                    Category { id: "stat.OT", name: "Other Statistics" },
                    Category { id: "stat.TH", name: "Statistics Theory" },
                ],
            },
        ],
    },
];
