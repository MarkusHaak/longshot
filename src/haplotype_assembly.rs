// calls HapCUT2 as a static library
use variants_and_fragments::*;
use bio::stats::{LogProb, Prob, PHREDProb};
use std::collections::{HashMap, HashSet};
use std::char::from_digit;

pub fn separate_reads_by_haplotype(flist: &Vec<Fragment>, threshold: LogProb) -> (HashSet<String>, HashSet<String>) {

    let mut h1 = HashSet::new();
    let mut h2 = HashSet::new();

    for ref f in flist {
        let total: LogProb = LogProb::ln_add_exp(f.p_read_hap[0],f.p_read_hap[1]);
        let p_read_hap0: LogProb = f.p_read_hap[0] - total;
        let p_read_hap1: LogProb = f.p_read_hap[1] - total;

        if p_read_hap0 > threshold {
            h1.insert(f.id.clone());
        } else if p_read_hap1 > threshold {
            h2.insert(f.id.clone());
        }
    }

    (h1,h2)
}

pub fn generate_flist_buffer(flist: &Vec<Fragment>, phase_variant: &Vec<bool>, max_p_miscall: f64) -> Vec<Vec<u8>> {
    let mut buffer: Vec<Vec<u8>> = vec![];
    for frag in flist {
        let mut prev_call = phase_variant.len() + 1;
        let mut quals: Vec<u8> = vec![];
        let mut blocks = 0;
        let mut n_calls = 0;

        for c in frag.clone().calls {
            if phase_variant[c.var_ix] && c.qual < LogProb::from(Prob(max_p_miscall)) {
                n_calls += 1;
                if prev_call > phase_variant.len() || c.var_ix - prev_call != 1 {
                    blocks += 1;
                }
                prev_call = c.var_ix
            }
        }
        if n_calls < 2 {
            continue;
        }

        let mut line: Vec<u8> = vec![];
        for u in blocks.to_string().into_bytes() {
            line.push(u as u8);
        }
        line.push(' ' as u8);

        for u in frag.id.clone().into_bytes() {
            line.push(u as u8);
        }
        line.push(' ' as u8);

        let mut prev_call = phase_variant.len() + 1;

        for c in frag.clone().calls {
            if phase_variant[c.var_ix] && c.qual < LogProb::from(Prob(max_p_miscall)){
                if prev_call < c.var_ix && c.var_ix - prev_call == 1 {
                    line.push(from_digit(c.allele as u32, 10).unwrap() as u8)
                } else {
                    line.push(' ' as u8);
                    for u in (c.var_ix + 1).to_string().into_bytes() {
                        line.push(u as u8);
                    }
                    line.push(' ' as u8);
                    line.push(from_digit(c.allele as u32, 10).unwrap() as u8)
                }
                let mut qint = *PHREDProb::from(c.qual) as u32 + 33;
                if qint > 126 {
                    qint = 126;
                }
                quals.push(qint as u8);
                prev_call = c.var_ix
            }
        }
        line.push(' ' as u8);
        line.append(&mut quals);
        //line.push('\n' as u8);
        line.push('\0' as u8);

        let mut charline: Vec<char> = vec![];
        for u in line.clone() {
            charline.push(u as char)
        }

        //println!("{}", charline.iter().collect::<String>());

        buffer.push(line);
    }
    buffer
}

extern "C" {
    fn hapcut2(fragmentbuffer: *const *const u8,
               variantbuffer: *const *const u8,
               fragments: usize,
               snps: usize,
               hap1: *mut u8,
               phase_sets: *mut i32);
}

pub fn call_hapcut2(frag_buffer: &Vec<Vec<u8>>,
                    vcf_buffer: &Vec<Vec<u8>>,
                    fragments: usize,
                    snps: usize,
                    hap1: &mut Vec<u8>,
                    phase_sets: &mut Vec<i32>) {

    unsafe {
        let mut frag_ptrs: Vec<*const u8> = Vec::with_capacity(frag_buffer.len());
        let mut vcf_ptrs: Vec<*const u8> = Vec::with_capacity(vcf_buffer.len());

        for line in frag_buffer {
            frag_ptrs.push(line.as_ptr());
        }

        for line in vcf_buffer {
            vcf_ptrs.push(line.as_ptr());
        }

        hapcut2(frag_ptrs.as_ptr(),
                vcf_ptrs.as_ptr(),
                fragments,
                snps,
                hap1.as_mut_ptr(),
                phase_sets.as_mut_ptr());
    }
}

pub fn calculate_mec(flist: &Vec<Fragment>, varlist: &mut VarList, max_p_miscall: f64) {

    let hap_ixs = vec![0, 1];
    let ln_max_p_miscall = LogProb::from(Prob(max_p_miscall));

    for mut var in &mut varlist.lst {
        var.mec = 0;
        var.mec_frac_variant = 0.0;
        var.mec_frac_block = 0.0;
    }

    for f in 0..flist.len() {

        let mut mismatched_vars: Vec<Vec<usize>> = vec![vec![], vec![]];

        for &hap_ix in &hap_ixs {
            for call in &flist[f].calls {
                if call.qual < ln_max_p_miscall {
                    let var = &varlist.lst[call.var_ix]; // the variant that the fragment call covers

                    if var.phase_set == None {
                        continue; // only care about phased variants.
                    }

                    let hap_allele = if hap_ix == 0 {var.genotype.0} else {var.genotype.1};

                    // read allele matches haplotype allele
                    if call.allele != hap_allele {
                        mismatched_vars[hap_ix].push(call.var_ix);
                    }
                }
            }
        }

        let min_error_hap = if mismatched_vars[0].len() < mismatched_vars[1].len() { 0 } else { 1 };

        for &ix in &mismatched_vars[min_error_hap] {
            varlist.lst[ix].mec += 1;
        }
    }

    let mut block_mec: HashMap<usize, usize> = HashMap::new();
    let mut block_total: HashMap<usize, usize> = HashMap::new();

    for mut var in &mut varlist.lst {
        match var.phase_set {
            Some(ps) => {
                *block_mec.entry(ps).or_insert(0) += var.mec;
                *block_total.entry(ps).or_insert(0) += var.allele_counts.iter().sum::<usize>();
            }
            None => {}
        }
    }

    for mut var in &mut varlist.lst {
        match var.phase_set {
            Some(ps) => {
                var.mec_frac_block = *block_mec.get(&ps).unwrap() as f64 / *block_total.get(&ps).unwrap() as f64;
                var.mec_frac_variant = var.mec as f64 / var.allele_counts.iter().sum::<usize>() as f64;
            }
            None => {}
        }
    }
}
